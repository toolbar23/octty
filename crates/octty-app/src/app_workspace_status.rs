use super::*;
use notify::Watcher as _;

const WORKSPACE_STATUS_REFRESH_DEBOUNCE: Duration = Duration::from_millis(180);

impl OcttyApp {
    pub(crate) fn ensure_workspace_watchers(&mut self) {
        let desired = self
            .workspaces
            .iter()
            .filter(|workspace| has_recorded_workspace_path(&workspace.workspace_path))
            .map(|workspace| (workspace.id.clone(), workspace.workspace_path.clone()))
            .collect::<HashMap<_, _>>();

        self.workspace_watchers.retain(|workspace_id, watcher| {
            desired
                .get(workspace_id)
                .is_some_and(|path| path == &watcher.path)
        });

        for workspace in self.workspaces.clone() {
            if !has_recorded_workspace_path(&workspace.workspace_path)
                || self.workspace_watchers.contains_key(&workspace.id)
            {
                continue;
            }
            match create_workspace_watcher(&workspace, self.workspace_watch_tx.clone()) {
                Ok(watcher) => {
                    self.workspace_watchers
                        .insert(workspace.id.clone(), watcher);
                }
                Err(error) => {
                    eprintln!(
                        "[octty-app] failed to watch workspace {}: {error}",
                        workspace.display_name_or_workspace_name()
                    );
                }
            }
        }
    }

    pub(crate) fn schedule_workspace_watch_notifications(&mut self, cx: &mut Context<Self>) {
        if self.workspace_watch_notifications_active {
            return;
        }
        let Some(mut watch_rx) = self.workspace_watch_rx.take() else {
            return;
        };

        self.workspace_watch_notifications_active = true;
        cx.spawn(async move |this, cx| {
            while let Some(workspace_id) = watch_rx.next().await {
                let mut workspace_ids = BTreeSet::from([workspace_id]);
                while let Ok(workspace_id) = watch_rx.try_recv() {
                    workspace_ids.insert(workspace_id);
                }
                let _ = this.update(cx, |app, cx| {
                    for workspace_id in workspace_ids {
                        app.schedule_workspace_status_refresh(workspace_id, cx);
                    }
                });
            }

            let _ = this.update(cx, |app, _cx| {
                app.workspace_watch_notifications_active = false;
            });
        })
        .detach();
    }

    pub(crate) fn schedule_workspace_status_refresh(
        &mut self,
        workspace_id: String,
        cx: &mut Context<Self>,
    ) {
        self.workspace_status_refresh_due_at.insert(
            workspace_id.clone(),
            Instant::now() + WORKSPACE_STATUS_REFRESH_DEBOUNCE,
        );
        if !self
            .workspace_status_refresh_timer_active
            .insert(workspace_id.clone())
        {
            return;
        }

        cx.spawn(async move |this, cx| {
            loop {
                let Some(delay) = this
                    .update(cx, |app, _cx| {
                        app.workspace_status_refresh_due_at
                            .get(&workspace_id)
                            .copied()
                            .map(|due_at| due_at.saturating_duration_since(Instant::now()))
                    })
                    .ok()
                    .flatten()
                else {
                    break;
                };
                cx.background_executor().timer(delay).await;
                let ready = this
                    .update(cx, |app, _cx| {
                        let now = Instant::now();
                        match app
                            .workspace_status_refresh_due_at
                            .get(&workspace_id)
                            .copied()
                        {
                            Some(due_at) if due_at > now => false,
                            Some(_) => {
                                app.workspace_status_refresh_due_at.remove(&workspace_id);
                                true
                            }
                            None => true,
                        }
                    })
                    .unwrap_or(false);
                if ready {
                    break;
                }
            }

            let Some((store, workspace)) = this
                .update(cx, |app, _cx| {
                    app.workspace_status_refresh_timer_active
                        .remove(&workspace_id);
                    app.workspaces
                        .iter()
                        .find(|workspace| workspace.id == workspace_id)
                        .cloned()
                        .map(|workspace| (app.store.clone(), workspace))
                })
                .ok()
                .flatten()
            else {
                return;
            };

            let result = match gpui_tokio::Tokio::spawn_result(
                cx,
                refresh_workspace_status(store, workspace),
            ) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |app, cx| match result {
                Ok(workspace) => {
                    if let Some(index) = app
                        .workspaces
                        .iter()
                        .position(|item| item.id == workspace.id)
                    {
                        app.workspaces[index] = workspace;
                        cx.notify();
                    }
                }
                Err(error) => {
                    eprintln!("[octty-app] workspace status refresh failed: {error:#}");
                }
            });
        })
        .detach();
    }
}

pub(crate) fn merge_jj_workspace_status(
    existing: &WorkspaceStatus,
    jj_status: WorkspaceStatus,
) -> WorkspaceStatus {
    WorkspaceStatus {
        unread_notes: existing.unread_notes,
        active_agent_count: existing.active_agent_count,
        agent_attention_state: existing.agent_attention_state.clone(),
        ..jj_status
    }
}

pub(crate) fn reset_jj_workspace_status(existing: &WorkspaceStatus) -> WorkspaceStatus {
    WorkspaceStatus {
        unread_notes: existing.unread_notes,
        active_agent_count: existing.active_agent_count,
        agent_attention_state: existing.agent_attention_state.clone(),
        recent_activity_at: now_ms(),
        ..WorkspaceStatus::default()
    }
}

async fn refresh_workspace_status(
    store: Arc<TursoStore>,
    workspace: WorkspaceSummary,
) -> anyhow::Result<WorkspaceSummary> {
    let mut next = workspace.clone();
    next.status = if has_recorded_workspace_path(&workspace.workspace_path) {
        merge_jj_workspace_status(
            &workspace.status,
            read_workspace_status(&workspace.workspace_path, &workspace.workspace_name).await?,
        )
    } else {
        reset_jj_workspace_status(&workspace.status)
    };
    next.updated_at = now_ms();
    store.upsert_workspace(&next).await?;
    Ok(next)
}

fn create_workspace_watcher(
    workspace: &WorkspaceSummary,
    tx: mpsc::UnboundedSender<String>,
) -> notify::Result<WorkspacePathWatcher> {
    let workspace_id = workspace.id.clone();
    let workspace_name = workspace.display_name_or_workspace_name().to_owned();
    let fragments = Arc::new(parse_workspace_watch_ignore_fragments());
    let watcher_workspace_id = workspace_id.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) => {
                if event
                    .paths
                    .iter()
                    .any(|path| !should_ignore_workspace_watch_path(path, &fragments))
                {
                    let _ = tx.unbounded_send(watcher_workspace_id.clone());
                }
            }
            Err(error) => {
                eprintln!("[octty-app] workspace watch error for {workspace_name}: {error}");
            }
        })?;
    watcher.watch(
        Path::new(&workspace.workspace_path),
        notify::RecursiveMode::Recursive,
    )?;
    Ok(WorkspacePathWatcher {
        path: workspace.workspace_path.clone(),
        _watcher: watcher,
    })
}

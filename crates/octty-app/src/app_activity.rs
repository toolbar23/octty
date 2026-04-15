use super::*;

impl OcttyApp {
    pub(crate) fn active_workspace(&self) -> Option<&WorkspaceSummary> {
        self.active_workspace_index
            .and_then(|index| self.workspaces.get(index))
    }

    pub(crate) fn record_pane_seen(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        seen_at_ms: i64,
        cx: &mut Context<Self>,
    ) {
        let key = (workspace_id.to_owned(), pane_id.to_owned());
        let activity = self
            .pane_activity
            .entry(key.clone())
            .or_insert_with(|| PaneActivity::new(workspace_id, pane_id, seen_at_ms));
        activity.record_seen(seen_at_ms);
        self.pending_pane_activity_persistence
            .insert(key, activity.clone());
        self.schedule_pane_activity_persistence(cx);
    }

    pub(crate) fn record_active_pane_seen(&mut self, cx: &mut Context<Self>) {
        let Some(workspace_id) = self
            .active_workspace()
            .map(|workspace| workspace.id.clone())
        else {
            return;
        };
        let Some(pane_id) = self
            .active_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active_pane_id.clone())
        else {
            return;
        };
        self.record_pane_seen(&workspace_id, &pane_id, now_ms(), cx);
    }

    pub(crate) fn delete_pane_activity(
        &mut self,
        workspace_id: &str,
        pane_id: &str,
        cx: &mut Context<Self>,
    ) {
        let key = (workspace_id.to_owned(), pane_id.to_owned());
        self.pane_activity.remove(&key);
        self.pending_pane_activity_persistence.remove(&key);
        let store = self.store.clone();
        let workspace_id = workspace_id.to_owned();
        let pane_id = pane_id.to_owned();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                store.delete_pane_activity(&workspace_id, &pane_id).await?;
                Ok(())
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                let _ = this.update(cx, |app, cx| {
                    app.status =
                        format!("Closed pane, but failed to remove activity: {error:#}").into();
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn schedule_pane_activity_persistence(&mut self, cx: &mut Context<Self>) {
        if self.pane_activity_persist_active {
            return;
        }
        self.pane_activity_persist_active = true;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(PANE_ACTIVITY_PERSIST_DELAY)
                    .await;
                let Some((store, activities)) = this
                    .update(cx, |app, _cx| {
                        let activities = std::mem::take(&mut app.pending_pane_activity_persistence)
                            .into_values()
                            .collect::<Vec<_>>();
                        if activities.is_empty() {
                            app.pane_activity_persist_active = false;
                            None
                        } else {
                            Some((app.store.clone(), activities))
                        }
                    })
                    .ok()
                    .flatten()
                else {
                    break;
                };

                let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                    store.upsert_pane_activities(&activities).await?;
                    Ok(())
                }) {
                    Ok(task) => task.await,
                    Err(error) => Err(error),
                };

                let should_continue = this
                    .update(cx, |app, cx| {
                        if let Err(error) = result {
                            app.status = format!("Failed to save pane activity: {error:#}").into();
                            cx.notify();
                        }
                        if app.pending_pane_activity_persistence.is_empty() {
                            app.pane_activity_persist_active = false;
                            false
                        } else {
                            true
                        }
                    })
                    .unwrap_or(false);
                if !should_continue {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn schedule_pane_activity_reconciliation(&mut self, cx: &mut Context<Self>) {
        if self.pane_activity_reconcile_active {
            return;
        }
        self.pane_activity_reconcile_active = true;
        cx.spawn(async move |this, cx| {
            loop {
                let Some((store, active_workspace_id)) = this
                    .update(cx, |app, _cx| {
                        (
                            app.store.clone(),
                            app.active_workspace().map(|workspace| workspace.id.clone()),
                        )
                    })
                    .ok()
                else {
                    break;
                };
                let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                    reconcile_pane_activity(store, active_workspace_id).await
                }) {
                    Ok(task) => task.await,
                    Err(error) => Err(error),
                };
                let update_ok = this.update(cx, |app, cx| match result {
                    Ok(activities) => {
                        for activity in activities {
                            app.pane_activity.insert(
                                (activity.workspace_id.clone(), activity.pane_id.clone()),
                                activity,
                            );
                        }
                        cx.notify();
                    }
                    Err(error) => {
                        eprintln!("[octty-app] pane activity reconciliation failed: {error:#}");
                    }
                });
                if update_ok.is_err() {
                    break;
                }
                cx.background_executor()
                    .timer(PANE_ACTIVITY_RECONCILE_INTERVAL)
                    .await;
            }
        })
        .detach();
    }
}

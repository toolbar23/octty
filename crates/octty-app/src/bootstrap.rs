#[derive(Clone)]
struct BootstrapState {
    status: String,
    project_roots: Vec<ProjectRootRecord>,
    workspaces: Vec<WorkspaceSummary>,
    active_workspace_index: Option<usize>,
    active_snapshot: Option<WorkspaceSnapshot>,
    pane_activity: Vec<PaneActivity>,
}

async fn load_bootstrap(auto_seed_current_repo: bool) -> anyhow::Result<BootstrapState> {
    load_bootstrap_with_active(auto_seed_current_repo, None).await
}

async fn load_bootstrap_with_active(
    auto_seed_current_repo: bool,
    active_workspace_id: Option<String>,
) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(default_store_path()).await?;
    let mut roots = store.list_project_roots().await?;
    if roots.is_empty() && auto_seed_current_repo {
        if let Ok(root_path) = resolve_repo_root(std::env::current_dir()?).await {
            let root = project_root_from_path(&root_path);
            store.upsert_project_root(&root).await?;
            roots.push(root);
        }
    }

    let mut errors = Vec::new();
    let mut workspaces = Vec::new();
    let existing_workspaces = store
        .list_workspaces()
        .await?
        .into_iter()
        .map(|workspace| (workspace.id.clone(), workspace))
        .collect::<HashMap<_, _>>();
    for root in &roots {
        match discover_workspaces(root).await {
            Ok(discovered) => {
                for mut workspace in discovered {
                    let now = now_ms();
                    if let Some(existing) = existing_workspaces.get(&workspace.id) {
                        workspace.display_name = existing.display_name.clone();
                        workspace.created_at = existing.created_at;
                        workspace.last_opened_at = existing.last_opened_at;
                    } else if workspace.created_at == 0 {
                        workspace.created_at = now;
                    }
                    workspace.updated_at = now;
                    if has_recorded_workspace_path(&workspace.workspace_path) {
                        match read_workspace_status(&workspace.workspace_path).await {
                            Ok(status) => workspace.status = status,
                            Err(error) => errors.push(format!(
                                "{}: failed to read status: {error}",
                                workspace.workspace_name
                            )),
                        }
                    }
                    store.upsert_workspace(&workspace).await?;
                    workspaces.push(workspace);
                }
            }
            Err(error) => errors.push(format!(
                "{}: failed to discover workspaces: {error}",
                root.root_path
            )),
        }
    }

    if workspaces.is_empty() {
        workspaces = store.list_workspaces().await?;
    }

    let active_workspace_index = if workspaces.is_empty() {
        None
    } else {
        active_workspace_id
            .as_deref()
            .and_then(|workspace_id| {
                workspaces
                    .iter()
                    .position(|workspace| workspace.id == workspace_id)
            })
            .or(Some(0))
    };
    let active_snapshot = match active_workspace_index {
        Some(index) => Some(load_workspace_snapshot(&store, &workspaces[index]).await?),
        None => None,
    };
    let pane_activity = store.list_pane_activity().await?;

    let status = if workspaces.is_empty() {
        "No project roots yet. Run from inside a JJ repo to auto-seed the first root.".to_owned()
    } else if errors.is_empty() {
        format!("Loaded {} JJ workspace(s).", workspaces.len())
    } else {
        format!(
            "Loaded {} JJ workspace(s), with {} refresh warning(s).",
            workspaces.len(),
            errors.len()
        )
    };

    Ok(BootstrapState {
        status,
        project_roots: roots,
        workspaces,
        active_workspace_index,
        active_snapshot,
        pane_activity,
    })
}

async fn create_workspace_for_root_and_reload(
    store_path: PathBuf,
    root: ProjectRootRecord,
) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(store_path).await?;
    let (workspace_name, destination_path) = next_workspace_defaults(&store, &root).await?;
    jj_create_workspace(&root.root_path, &destination_path, &workspace_name).await?;
    let root_path = tokio::fs::canonicalize(&root.root_path).await?;
    let workspace_id =
        octty_jj::workspace_id_for(&root_path.to_string_lossy(), workspace_name.as_str());
    load_bootstrap_with_active(true, Some(workspace_id)).await
}

async fn add_project_root_and_reload(
    store_path: PathBuf,
    selected_path: PathBuf,
    active_workspace_id: Option<String>,
) -> anyhow::Result<BootstrapState> {
    let root_path = resolve_repo_root(selected_path).await?;
    let root = project_root_from_path(&root_path);
    let store = TursoStore::open(store_path).await?;
    store.upsert_project_root(&root).await?;

    let workspace_id = discover_workspaces(&root)
        .await
        .ok()
        .and_then(|workspaces| workspaces.first().map(|workspace| workspace.id.clone()))
        .or(active_workspace_id);
    load_bootstrap_with_active(true, workspace_id).await
}

async fn rename_project_root_and_reload(
    store_path: PathBuf,
    root_id: String,
    display_name: String,
    active_workspace_id: Option<String>,
) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(store_path).await?;
    store
        .update_project_root_display_name(&root_id, &display_name)
        .await?;
    store
        .update_workspace_project_display_name(&root_id, &display_name)
        .await?;
    load_bootstrap_with_active(true, active_workspace_id).await
}

async fn rename_workspace_and_reload(
    store_path: PathBuf,
    workspace_id: String,
    display_name: String,
    active_workspace_id: Option<String>,
) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(store_path).await?;
    store
        .update_workspace_display_name(&workspace_id, &display_name)
        .await?;
    load_bootstrap_with_active(true, active_workspace_id).await
}

async fn remove_project_root_and_reload(
    store_path: PathBuf,
    root_id: String,
    active_workspace_id: Option<String>,
) -> anyhow::Result<BootstrapState> {
    let store = TursoStore::open(store_path).await?;
    store.delete_project_root(&root_id).await?;
    load_bootstrap_with_active(true, active_workspace_id).await
}

async fn forget_workspace_and_reload(
    store_path: PathBuf,
    workspace: WorkspaceSummary,
    active_workspace_id: Option<String>,
    delete_directory: bool,
) -> anyhow::Result<BootstrapState> {
    jj_forget_workspace(&workspace.root_path, &workspace.workspace_name).await?;
    if delete_directory {
        delete_workspace_directory(&workspace).await?;
    }
    let store = TursoStore::open(store_path).await?;
    store.delete_workspace(&workspace.id).await?;
    load_bootstrap_with_active(true, active_workspace_id).await
}

async fn next_workspace_defaults(
    store: &TursoStore,
    root: &ProjectRootRecord,
) -> anyhow::Result<(String, String)> {
    let base_directory = default_workspace_directory(root);
    tokio::fs::create_dir_all(&base_directory).await?;
    let existing = store
        .list_workspaces()
        .await?
        .into_iter()
        .filter(|workspace| workspace.root_id == root.id)
        .map(|workspace| workspace.workspace_name)
        .collect::<BTreeSet<_>>();
    for attempt in 1..=200 {
        let workspace_name = format!("workspace-{attempt}");
        let destination = base_directory.join(&workspace_name);
        if existing.contains(&workspace_name) || tokio::fs::try_exists(&destination).await? {
            continue;
        }
        return Ok((workspace_name, destination.to_string_lossy().to_string()));
    }
    anyhow::bail!(
        "could not find an unused workspace name under {}",
        base_directory.display()
    );
}

fn default_workspace_directory(root: &ProjectRootRecord) -> PathBuf {
    let repo_name = Path::new(&root.root_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("repo");
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("workspaces")
        .join(repo_name)
}

async fn delete_workspace_directory(workspace: &WorkspaceSummary) -> anyhow::Result<()> {
    if !has_recorded_workspace_path(&workspace.workspace_path) {
        anyhow::bail!(
            "workspace {} was forgotten, but its directory path is not recorded",
            workspace.workspace_name
        );
    }
    let workspace_path = tokio::fs::canonicalize(&workspace.workspace_path)
        .await
        .unwrap_or_else(|_| PathBuf::from(&workspace.workspace_path));
    let root_path = tokio::fs::canonicalize(&workspace.root_path)
        .await
        .unwrap_or_else(|_| PathBuf::from(&workspace.root_path));
    if workspace_path == root_path {
        anyhow::bail!(
            "refusing to delete the repo root for workspace {}",
            workspace.workspace_name
        );
    }
    tokio::fs::remove_dir_all(workspace_path).await?;
    Ok(())
}

fn sanitize_display_name(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

async fn load_workspace_snapshot(
    store: &TursoStore,
    workspace: &WorkspaceSummary,
) -> anyhow::Result<WorkspaceSnapshot> {
    if let Some(snapshot) = store.get_snapshot(&workspace.id).await?
        && snapshot.layout_version == LAYOUT_VERSION
    {
        return Ok(snapshot);
    }

    let snapshot = create_default_snapshot(workspace.id.clone());
    store.save_snapshot(&snapshot).await?;
    Ok(snapshot)
}

async fn reconcile_pane_activity(store_path: PathBuf) -> anyhow::Result<Vec<PaneActivity>> {
    let store = TursoStore::open(store_path).await?;
    let mut activity_by_pane = pane_activity_map(store.list_pane_activity().await?);
    let snapshots = store.list_snapshots().await?;
    let mut updated = Vec::new();

    for snapshot in snapshots {
        for (pane_id, pane) in snapshot.panes {
            let PanePayload::Terminal(payload) = pane.payload else {
                continue;
            };
            let spec = TerminalSessionSpec {
                workspace_id: snapshot.workspace_id.clone(),
                pane_id: pane_id.clone(),
                kind: payload.kind,
                cwd: payload.cwd,
                cols: 120,
                rows: 40,
            };
            let session_name = stable_tmux_session_name(&spec);
            let Some(tmux_activity) = tmux_session_activity(&session_name).await? else {
                continue;
            };
            let screen = capture_tmux_pane_by_session(&session_name)
                .await
                .ok()
                .map(|screen| screen_fingerprint(&screen));
            let activity_at_s = tmux_activity
                .window_activity_at_s
                .or(tmux_activity.session_activity_at_s);
            let now = now_ms();
            let key = (snapshot.workspace_id.clone(), pane_id.clone());
            let activity = activity_by_pane
                .entry(key)
                .or_insert_with(|| PaneActivity::new(snapshot.workspace_id.clone(), pane_id, now));
            activity.record_tmux_observation(now, activity_at_s, screen);
            updated.push(activity.clone());
        }
    }

    store.upsert_pane_activities(&updated).await?;
    Ok(updated)
}

use octty_core::{
    PaneActivity, PaneType, ProjectRootRecord, SessionSnapshot, SessionState, TerminalKind,
    WorkspaceBookmarkRelation, WorkspaceState, WorkspaceStatus, WorkspaceSummary, add_pane,
    create_default_snapshot, create_pane_state,
};

use crate::TursoStore;

#[tokio::test]
async fn migrates_and_round_trips_project_roots() {
    let store = TursoStore::open_memory().await.unwrap();
    let root = ProjectRootRecord {
        id: "root-1".to_owned(),
        root_path: "/tmp/repo".to_owned(),
        display_name: "repo".to_owned(),
        created_at: 1,
        updated_at: 2,
    };

    store.upsert_project_root(&root).await.unwrap();

    assert_eq!(store.list_project_roots().await.unwrap(), vec![root]);
}

#[tokio::test]
async fn round_trips_workspace_snapshots() {
    let store = TursoStore::open_memory().await.unwrap();
    let snapshot = add_pane(
        create_default_snapshot("workspace-1"),
        create_pane_state(PaneType::Shell, "/tmp/repo", None),
    );

    store.save_snapshot(&snapshot).await.unwrap();

    assert_eq!(
        store.get_snapshot("workspace-1").await.unwrap(),
        Some(snapshot)
    );
}

#[tokio::test]
async fn round_trips_workspace_summaries() {
    let store = TursoStore::open_memory().await.unwrap();
    let root = ProjectRootRecord {
        id: "root-1".to_owned(),
        root_path: "/tmp/repo".to_owned(),
        display_name: "repo".to_owned(),
        created_at: 1,
        updated_at: 2,
    };
    store.upsert_project_root(&root).await.unwrap();
    let workspace = WorkspaceSummary {
        id: "workspace-1".to_owned(),
        root_id: root.id,
        root_path: "/tmp/repo".to_owned(),
        project_display_name: "repo".to_owned(),
        workspace_name: "default".to_owned(),
        display_name: "default".to_owned(),
        workspace_path: "/tmp/repo".to_owned(),
        status: WorkspaceStatus {
            workspace_state: WorkspaceState::Draft,
            has_working_copy_changes: true,
            bookmarks: vec!["main".to_owned()],
            bookmark_relation: WorkspaceBookmarkRelation::Exact,
            ..WorkspaceStatus::default()
        },
        created_at: 3,
        updated_at: 4,
        last_opened_at: 5,
    };

    store.upsert_workspace(&workspace).await.unwrap();

    assert_eq!(store.list_workspaces().await.unwrap(), vec![workspace]);
}

#[tokio::test]
async fn renames_workspace_records_and_rekeys_saved_state() {
    let store = TursoStore::open_memory().await.unwrap();
    let root = ProjectRootRecord {
        id: "root-1".to_owned(),
        root_path: "/tmp/repo".to_owned(),
        display_name: "repo".to_owned(),
        created_at: 1,
        updated_at: 2,
    };
    store.upsert_project_root(&root).await.unwrap();
    let workspace = WorkspaceSummary {
        id: "workspace-1".to_owned(),
        root_id: root.id,
        root_path: "/tmp/repo".to_owned(),
        project_display_name: "repo".to_owned(),
        workspace_name: "default".to_owned(),
        display_name: "default".to_owned(),
        workspace_path: "/tmp/repo".to_owned(),
        status: WorkspaceStatus::default(),
        created_at: 3,
        updated_at: 4,
        last_opened_at: 5,
    };
    store.upsert_workspace(&workspace).await.unwrap();
    store
        .save_snapshot(&create_default_snapshot("workspace-1"))
        .await
        .unwrap();
    store
        .upsert_session_state(&SessionSnapshot {
            id: "tmux-session-1".to_owned(),
            workspace_id: "workspace-1".to_owned(),
            pane_id: "pane-1".to_owned(),
            kind: TerminalKind::Shell,
            cwd: "/tmp/repo".to_owned(),
            command: "".to_owned(),
            buffer: "hello".to_owned(),
            screen: None,
            state: SessionState::Live,
            exit_code: None,
            embedded_session: None,
            embedded_session_correlation_id: None,
            agent_attention_state: None,
        })
        .await
        .unwrap();
    store
        .upsert_pane_activity(&PaneActivity::new("workspace-1", "pane-1", 1_000))
        .await
        .unwrap();

    store
        .rename_workspace("workspace-1", "workspace-2", "review", "review")
        .await
        .unwrap();

    let workspaces = store.list_workspaces().await.unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].id, "workspace-2");
    assert_eq!(workspaces[0].workspace_name, "review");
    assert_eq!(workspaces[0].display_name, "review");
    assert!(store.get_snapshot("workspace-1").await.unwrap().is_none());
    assert_eq!(
        store
            .get_snapshot("workspace-2")
            .await
            .unwrap()
            .unwrap()
            .workspace_id,
        "workspace-2"
    );
    assert_eq!(
        store
            .get_session_state_by_pane("pane-1")
            .await
            .unwrap()
            .unwrap()
            .workspace_id,
        "workspace-2"
    );
    assert_eq!(
        store.list_pane_activity().await.unwrap()[0].workspace_id,
        "workspace-2"
    );
}

#[tokio::test]
async fn round_trips_session_state() {
    let store = TursoStore::open_memory().await.unwrap();
    let session = SessionSnapshot {
        id: "tmux-session-1".to_owned(),
        workspace_id: "workspace-1".to_owned(),
        pane_id: "pane-1".to_owned(),
        kind: TerminalKind::Shell,
        cwd: "/tmp/repo".to_owned(),
        command: "".to_owned(),
        buffer: "hello".to_owned(),
        screen: None,
        state: SessionState::Live,
        exit_code: None,
        embedded_session: None,
        embedded_session_correlation_id: None,
        agent_attention_state: None,
    };

    store.upsert_session_state(&session).await.unwrap();

    assert_eq!(
        store.get_session_state_by_pane("pane-1").await.unwrap(),
        Some(session)
    );
}

#[tokio::test]
async fn round_trips_pane_activity() {
    let store = TursoStore::open_memory().await.unwrap();
    let mut activity = PaneActivity::new("workspace-1", "pane-1", 1_000);
    activity.record_activity(2_000, Some(2), Some("screen-a".to_owned()));
    activity.record_seen(3_000);
    activity.record_tmux_observation(4_000, Some(4), Some("screen-b".to_owned()));

    store.upsert_pane_activity(&activity).await.unwrap();

    assert_eq!(store.list_pane_activity().await.unwrap(), vec![activity]);
}

#[tokio::test]
async fn lists_saved_snapshots() {
    let store = TursoStore::open_memory().await.unwrap();
    let snapshot = add_pane(
        create_default_snapshot("workspace-1"),
        create_pane_state(PaneType::Shell, "/tmp/repo", None),
    );
    store.save_snapshot(&snapshot).await.unwrap();

    assert_eq!(store.list_snapshots().await.unwrap(), vec![snapshot]);
}

#[tokio::test]
async fn creates_parent_directories_for_file_databases() {
    let tempdir = tempfile::tempdir().unwrap();
    let db_path = tempdir.path().join("nested").join("state.turso");

    let _store = TursoStore::open(&db_path).await.unwrap();

    assert!(db_path.exists());
}

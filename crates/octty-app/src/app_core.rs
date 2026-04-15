impl OcttyApp {
    fn new(bootstrap: BootstrapState, focus_handle: FocusHandle, cx: &mut Context<Self>) -> Self {
        let (terminal_snapshot_tx, terminal_snapshot_rx) = mpsc::unbounded();
        let pane_activity = pane_activity_map(bootstrap.pane_activity);
        let mut app = Self {
            status: bootstrap.status.into(),
            project_roots: bootstrap.project_roots,
            workspaces: bootstrap.workspaces,
            active_workspace_index: bootstrap.active_workspace_index,
            active_snapshot: bootstrap.active_snapshot,
            store_path: default_store_path(),
            focus_handle,
            pending_terminal_inputs: Vec::new(),
            terminal_flush_active: false,
            live_terminals: HashMap::new(),
            failed_live_terminals: BTreeSet::new(),
            terminal_snapshot_tx,
            terminal_snapshot_rx: Some(terminal_snapshot_rx),
            terminal_notifications_active: false,
            terminal_deferred_snapshot_timer_active: false,
            terminal_window_active: true,
            terminal_last_snapshot_notify_at: None,
            terminal_glyph_cache: Rc::new(RefCell::new(TerminalGlyphLayoutCache::default())),
            terminal_render_cache: Rc::new(RefCell::new(TerminalRenderCache::default())),
            sidebar_menu: None,
            sidebar_rename_dialog: None,
            toasts: VecDeque::new(),
            next_toast_id: 1,
            pane_activity,
            pending_pane_activity_persistence: HashMap::new(),
            pane_activity_persist_active: false,
            pane_activity_reconcile_active: false,
        };
        if app.status.starts_with("Startup failed:") {
            app.show_error(app.status.clone(), cx);
        }
        app.ensure_live_terminals_for_active_snapshot(cx);
        app
    }

    fn open_workspace(
        &mut self,
        action: &OpenWorkspaceShortcut,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.index >= self.workspaces.len() {
            return;
        }

        if self.active_workspace_index == Some(action.index) {
            if self.sidebar_menu.is_some() {
                self.sidebar_menu = None;
                cx.notify();
            }
            return;
        }

        self.sidebar_menu = None;
        self.active_workspace_index = Some(action.index);
        let workspace = self.workspaces[action.index].clone();
        let workspace_id = workspace.id.clone();
        let workspace_display_name = workspace.display_name_or_workspace_name().to_owned();
        self.active_snapshot = None;
        self.status = format!("Opening {workspace_display_name}...").into();
        cx.notify();

        let store_path = self.store_path.clone();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                let store = TursoStore::open(store_path).await?;
                load_workspace_snapshot(&store, &workspace).await
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };

            let _ = this.update(cx, |app, cx| {
                let still_active = app
                    .active_workspace()
                    .is_some_and(|workspace| workspace.id == workspace_id);
                if !still_active {
                    return;
                }
                match result {
                    Ok(snapshot) => {
                        app.active_snapshot = Some(snapshot);
                        app.ensure_live_terminals_for_active_snapshot(cx);
                        app.schedule_terminal_snapshot_notifications(cx);
                        app.record_active_pane_seen(cx);
                        app.status = format!("Opened {workspace_display_name}.").into();
                    }
                    Err(error) => {
                        app.show_error(
                            format!("Failed to open {workspace_display_name}: {error:#}"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn navigate_workspace(
        &mut self,
        action: &NavigateWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspaces.is_empty() {
            return;
        }

        let current = self
            .active_workspace_index
            .unwrap_or(0)
            .min(self.workspaces.len().saturating_sub(1));
        let target = match action.direction {
            WorkspaceNavigationDirection::Previous => {
                current.checked_sub(1).unwrap_or(self.workspaces.len() - 1)
            }
            WorkspaceNavigationDirection::Next => (current + 1) % self.workspaces.len(),
        };
        if target == current {
            return;
        }

        self.open_workspace(&OpenWorkspaceShortcut { index: target }, window, cx);
    }

    fn add_project_root(&mut self, _: &AddProjectRoot, _: &mut Window, cx: &mut Context<Self>) {
        let path_rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Add repository".into()),
        });
        let store_path = self.store_path.clone();
        let active_workspace_id = self
            .active_workspace()
            .map(|workspace| workspace.id.clone());
        self.status = "Choose a JJ repository directory...".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let selected_path = match path_rx.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => Ok(path),
                    None => Err(anyhow::anyhow!("no repository directory was selected")),
                },
                Ok(Ok(None)) => return,
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow::anyhow!("repository picker failed: {error}")),
            };
            let result = match selected_path {
                Ok(selected_path) => match gpui_tokio::Tokio::spawn_result(cx, async move {
                    add_project_root_and_reload(store_path, selected_path, active_workspace_id)
                        .await
                }) {
                    Ok(task) => task.await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            };

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(bootstrap) => app.apply_bootstrap(bootstrap, cx),
                    Err(error) => {
                        app.show_error(format!("Failed to add repository: {error:#}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create_workspace_for_root(
        &mut self,
        action: &CreateWorkspaceForRoot,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self
            .project_roots
            .iter()
            .find(|root| root.id == action.root_id)
            .cloned()
        else {
            return;
        };
        let store_path = self.store_path.clone();
        self.run_navigation_operation(
            format!("Creating workspace in {}...", root.display_name),
            async move { create_workspace_for_root_and_reload(store_path, root).await },
            cx,
        );
    }

    fn rename_project_root(
        &mut self,
        action: &RenameProjectRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self
            .project_roots
            .iter()
            .find(|root| root.id == action.root_id)
            .cloned()
        else {
            return;
        };
        self.open_sidebar_rename_dialog(
            SidebarRenameTarget::ProjectRoot(root.id),
            root.display_name,
            window,
            cx,
        );
    }

    fn remove_project_root(
        &mut self,
        action: &RemoveProjectRoot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self
            .project_roots
            .iter()
            .find(|root| root.id == action.root_id)
            .cloned()
        else {
            return;
        };
        let active_workspace_id = self
            .active_workspace()
            .filter(|workspace| workspace.root_id != root.id)
            .map(|workspace| workspace.id.clone());
        let store_path = self.store_path.clone();
        let answer = window.prompt(
            PromptLevel::Warning,
            "Forget this project root from Octty?",
            Some(root.root_path.as_str()),
            &["Forget", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                remove_project_root_and_reload(store_path, root.id, active_workspace_id).await
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(bootstrap) => app.apply_bootstrap(bootstrap, cx),
                    Err(error) => {
                        app.show_error(format!("Failed to forget project root: {error:#}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rename_workspace(
        &mut self,
        action: &RenameWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == action.workspace_id)
            .cloned()
        else {
            return;
        };
        self.open_sidebar_rename_dialog(
            SidebarRenameTarget::Workspace(workspace.id.clone()),
            workspace.display_name_or_workspace_name().to_owned(),
            window,
            cx,
        );
    }

    fn forget_workspace(
        &mut self,
        action: &ForgetWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_forget_workspace(action.workspace_id.clone(), false, window, cx);
    }

    fn delete_and_forget_workspace(
        &mut self,
        action: &DeleteAndForgetWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_and_forget_workspace(action.workspace_id.clone(), true, window, cx);
    }

    fn show_sidebar_menu(
        &mut self,
        entries: Vec<SidebarMenuEntry>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let app_entity = cx.entity();
        let menu = cx.new(|cx| SidebarMenuView::new(app_entity, entries, cx.focus_handle()));
        menu.update(cx, |menu, _| menu.focus_handle.focus(window));
        self.sidebar_menu = Some(SidebarMenuOverlay { position, menu });
        cx.notify();
    }

    fn dismiss_sidebar_menu(&mut self, cx: &mut Context<Self>) {
        self.sidebar_menu = None;
        cx.notify();
    }

    fn execute_sidebar_menu_action(
        &mut self,
        action: SidebarMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sidebar_menu = None;
        match action {
            SidebarMenuAction::CreateWorkspaceForRoot(root_id) => {
                self.create_workspace_for_root(&CreateWorkspaceForRoot { root_id }, window, cx);
            }
            SidebarMenuAction::RenameProjectRoot(root_id) => {
                self.rename_project_root(&RenameProjectRoot { root_id }, window, cx);
            }
            SidebarMenuAction::RemoveProjectRoot(root_id) => {
                self.remove_project_root(&RemoveProjectRoot { root_id }, window, cx);
            }
            SidebarMenuAction::RenameWorkspace(workspace_id) => {
                self.rename_workspace(&RenameWorkspace { workspace_id }, window, cx);
            }
            SidebarMenuAction::ForgetWorkspace(workspace_id) => {
                self.forget_workspace(&ForgetWorkspace { workspace_id }, window, cx);
            }
            SidebarMenuAction::DeleteAndForgetWorkspace(workspace_id) => {
                self.delete_and_forget_workspace(
                    &DeleteAndForgetWorkspace { workspace_id },
                    window,
                    cx,
                );
            }
        }
    }

    fn project_root_menu_entries(root_id: &str) -> Vec<SidebarMenuEntry> {
        vec![
            SidebarMenuEntry::item(
                "New Workspace",
                SidebarMenuAction::CreateWorkspaceForRoot(root_id.to_owned()),
            ),
            SidebarMenuEntry::item(
                "Rename",
                SidebarMenuAction::RenameProjectRoot(root_id.to_owned()),
            ),
            SidebarMenuEntry::Separator,
            SidebarMenuEntry::item(
                "Forget",
                SidebarMenuAction::RemoveProjectRoot(root_id.to_owned()),
            ),
        ]
    }

    fn workspace_menu_entries(
        workspace_id: &str,
        forget_disabled: bool,
        delete_disabled: bool,
    ) -> Vec<SidebarMenuEntry> {
        vec![
            SidebarMenuEntry::item(
                "Rename",
                SidebarMenuAction::RenameWorkspace(workspace_id.to_owned()),
            ),
            SidebarMenuEntry::Separator,
            SidebarMenuEntry::item_with_disabled(
                "Forget",
                SidebarMenuAction::ForgetWorkspace(workspace_id.to_owned()),
                forget_disabled,
            ),
            SidebarMenuEntry::item_with_disabled(
                "Delete and forget",
                SidebarMenuAction::DeleteAndForgetWorkspace(workspace_id.to_owned()),
                delete_disabled,
            ),
        ]
    }

    fn open_sidebar_rename_dialog(
        &mut self,
        target: SidebarRenameTarget,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = match target {
            SidebarRenameTarget::ProjectRoot(_) => "Rename repo",
            SidebarRenameTarget::Workspace(_) => "Rename workspace",
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        self.sidebar_menu = None;
        self.sidebar_rename_dialog = Some(SidebarRenameDialog {
            target,
            title: title.into(),
            input: input.clone(),
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn cancel_sidebar_rename_dialog(&mut self, cx: &mut Context<Self>) {
        self.sidebar_rename_dialog = None;
        cx.notify();
    }

    fn confirm_sidebar_rename_dialog(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = self.sidebar_rename_dialog.take() else {
            return;
        };
        let display_name = sanitize_display_name(&dialog.input.read(cx).value().to_string());
        if display_name.is_empty() {
            self.sidebar_rename_dialog = Some(dialog);
            self.show_error("Display name cannot be empty.", cx);
            return;
        }

        self.rename_sidebar_target(dialog.target, display_name, cx);
    }

    fn confirm_and_forget_workspace(
        &mut self,
        workspace_id: String,
        delete_directory: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .cloned()
        else {
            return;
        };
        if workspace.workspace_name == "default" {
            self.show_error("The default workspace cannot be forgotten.", cx);
            return;
        }
        let active_workspace_id = self
            .active_workspace()
            .filter(|active| active.id != workspace.id)
            .map(|active| active.id.clone());
        let store_path = self.store_path.clone();
        let message = if delete_directory {
            format!(
                "Delete the workspace directory and forget {}?",
                workspace.display_name_or_workspace_name()
            )
        } else {
            format!(
                "Forget {} from JJ and Octty?",
                workspace.display_name_or_workspace_name()
            )
        };
        let detail = delete_directory.then_some(workspace.workspace_path.as_str());
        let confirm_label = if delete_directory { "Delete" } else { "Forget" };
        let answer = window.prompt(
            PromptLevel::Warning,
            message.as_str(),
            detail,
            &[confirm_label, "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            let result = match gpui_tokio::Tokio::spawn_result(cx, async move {
                forget_workspace_and_reload(
                    store_path,
                    workspace,
                    active_workspace_id,
                    delete_directory,
                )
                .await
            }) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(bootstrap) => app.apply_bootstrap(bootstrap, cx),
                    Err(error) => {
                        app.show_error(format!("Failed to forget workspace: {error:#}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rename_sidebar_target(
        &mut self,
        target: SidebarRenameTarget,
        display_name: String,
        cx: &mut Context<Self>,
    ) {
        let store_path = self.store_path.clone();
        let active_workspace_id = self
            .active_workspace()
            .map(|workspace| workspace.id.clone());
        match target {
            SidebarRenameTarget::ProjectRoot(root_id) => {
                self.run_navigation_operation(
                    format!("Renaming repo to {display_name}..."),
                    async move {
                        rename_project_root_and_reload(
                            store_path,
                            root_id,
                            display_name,
                            active_workspace_id,
                        )
                        .await
                    },
                    cx,
                );
            }
            SidebarRenameTarget::Workspace(workspace_id) => {
                self.run_navigation_operation(
                    format!("Renaming workspace to {display_name}..."),
                    async move {
                        rename_workspace_and_reload(
                            store_path,
                            workspace_id,
                            display_name,
                            active_workspace_id,
                        )
                        .await
                    },
                    cx,
                );
            }
        }
    }

    fn run_navigation_operation<F>(
        &mut self,
        pending_status: String,
        operation: F,
        cx: &mut Context<Self>,
    ) where
        F: std::future::Future<Output = anyhow::Result<BootstrapState>> + Send + 'static,
    {
        self.status = pending_status.into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = match gpui_tokio::Tokio::spawn_result(cx, operation) {
                Ok(task) => task.await,
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(bootstrap) => app.apply_bootstrap(bootstrap, cx),
                    Err(error) => {
                        app.show_error(format!("Navigation action failed: {error:#}"), cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_bootstrap(&mut self, bootstrap: BootstrapState, cx: &mut Context<Self>) {
        self.status = bootstrap.status.into();
        self.project_roots = bootstrap.project_roots;
        self.workspaces = bootstrap.workspaces;
        self.active_workspace_index = bootstrap.active_workspace_index;
        self.active_snapshot = bootstrap.active_snapshot;
        self.pane_activity = pane_activity_map(bootstrap.pane_activity);
        self.ensure_live_terminals_for_active_snapshot(cx);
        self.schedule_terminal_snapshot_notifications(cx);
        self.schedule_pane_activity_reconciliation(cx);
        self.record_active_pane_seen(cx);
    }

    fn show_error(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let message = message.into();
        self.status = message.clone();
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.wrapping_add(1).max(1);
        self.toasts.push_back(AppToast { id, message });
        while self.toasts.len() > 4 {
            self.toasts.pop_front();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(6)).await;
            let _ = this.update(cx, |app, cx| app.dismiss_toast(id, cx));
        })
        .detach();
        cx.notify();
    }

    fn dismiss_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.toasts.len();
        self.toasts.retain(|toast| toast.id != id);
        if self.toasts.len() != before {
            cx.notify();
        }
    }
}

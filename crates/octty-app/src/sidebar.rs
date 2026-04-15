#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarRenameTarget {
    ProjectRoot(String),
    Workspace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarMenuAction {
    CreateWorkspaceForRoot(String),
    RenameProjectRoot(String),
    RemoveProjectRoot(String),
    RenameWorkspace(String),
    ForgetWorkspace(String),
    DeleteAndForgetWorkspace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SidebarMenuEntry {
    Item {
        label: SharedString,
        action: SidebarMenuAction,
        disabled: bool,
    },
    Separator,
}

impl SidebarMenuEntry {
    fn item(label: impl Into<SharedString>, action: SidebarMenuAction) -> Self {
        Self::item_with_disabled(label, action, false)
    }

    fn item_with_disabled(
        label: impl Into<SharedString>,
        action: SidebarMenuAction,
        disabled: bool,
    ) -> Self {
        Self::Item {
            label: label.into(),
            action,
            disabled,
        }
    }

    fn is_enabled_item(&self) -> bool {
        matches!(
            self,
            Self::Item {
                disabled: false,
                ..
            }
        )
    }

    fn action(&self) -> Option<SidebarMenuAction> {
        match self {
            Self::Item {
                action,
                disabled: false,
                ..
            } => Some(action.clone()),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct SidebarMenuOverlay {
    position: Point<Pixels>,
    menu: Entity<SidebarMenuView>,
}

struct SidebarRenameDialog {
    target: SidebarRenameTarget,
    title: SharedString,
    input: Entity<InputState>,
}

#[derive(Clone)]
struct AppToast {
    id: u64,
    message: SharedString,
}

struct SidebarMenuView {
    app: Entity<OcttyApp>,
    focus_handle: FocusHandle,
    entries: Vec<SidebarMenuEntry>,
    selected_index: Option<usize>,
}

impl SidebarMenuView {
    fn new(
        app: Entity<OcttyApp>,
        entries: Vec<SidebarMenuEntry>,
        focus_handle: FocusHandle,
    ) -> Self {
        let selected_index = entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| entry.is_enabled_item().then_some(index));
        Self {
            app,
            focus_handle,
            entries,
            selected_index,
        }
    }

    fn enabled_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.is_enabled_item().then_some(index))
            .collect()
    }

    fn select_up(&mut self, _: &SidebarMenuSelectUp, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.enabled_indices();
        if enabled.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(enabled[0]);
        let position = enabled
            .iter()
            .position(|index| *index == current)
            .unwrap_or(0);
        let next_position = position.checked_sub(1).unwrap_or(enabled.len() - 1);
        self.selected_index = Some(enabled[next_position]);
        cx.notify();
    }

    fn select_down(&mut self, _: &SidebarMenuSelectDown, _: &mut Window, cx: &mut Context<Self>) {
        let enabled = self.enabled_indices();
        if enabled.is_empty() {
            return;
        }
        let current = self.selected_index.unwrap_or(enabled[0]);
        let position = enabled
            .iter()
            .position(|index| *index == current)
            .unwrap_or(0);
        self.selected_index = Some(enabled[(position + 1) % enabled.len()]);
        cx.notify();
    }

    fn confirm(&mut self, _: &SidebarMenuConfirm, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.selected_index else {
            return;
        };
        self.execute_index(index, window, cx);
    }

    fn cancel(&mut self, _: &SidebarMenuCancel, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.app.update(cx, |app, cx| app.dismiss_sidebar_menu(cx));
    }

    fn execute_index(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self.entries.get(index).and_then(SidebarMenuEntry::action) else {
            return;
        };
        let _ = self.app.update(cx, |app, cx| {
            app.execute_sidebar_menu_action(action, window, cx);
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SidebarWorkspaceGroup {
    root: Option<ProjectRootRecord>,
    workspace_indices: Vec<usize>,
}

fn sidebar_workspace_groups(
    project_roots: &[ProjectRootRecord],
    workspaces: &[WorkspaceSummary],
) -> Vec<SidebarWorkspaceGroup> {
    let root_ids = project_roots
        .iter()
        .map(|root| root.id.clone())
        .collect::<BTreeSet<_>>();
    let mut groups = project_roots
        .iter()
        .map(|root| SidebarWorkspaceGroup {
            root: Some(root.clone()),
            workspace_indices: workspaces
                .iter()
                .enumerate()
                .filter_map(|(index, workspace)| (workspace.root_id == root.id).then_some(index))
                .collect(),
        })
        .collect::<Vec<_>>();
    let orphan_indices = workspaces
        .iter()
        .enumerate()
        .filter_map(|(index, workspace)| (!root_ids.contains(&workspace.root_id)).then_some(index))
        .collect::<Vec<_>>();
    if !orphan_indices.is_empty() {
        groups.push(SidebarWorkspaceGroup {
            root: None,
            workspace_indices: orphan_indices,
        });
    }
    groups
}

fn render_workspace_sidebar(
    project_roots: &[ProjectRootRecord],
    workspaces: &[WorkspaceSummary],
    active_workspace_index: Option<usize>,
    shortcut_labels: &HashMap<String, String>,
    pane_activity: &HashMap<(String, String), PaneActivity>,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let mut list = div().pt_3().px_4().flex().flex_col();
    if workspaces.is_empty() {
        return list.child(
            div()
                .py_2()
                .text_sm()
                .text_color(rgb(0x98a1ad))
                .child("No JJ workspaces discovered."),
        );
    }

    let groups = sidebar_workspace_groups(project_roots, workspaces);
    let group_count = groups.len();
    for (group_index, group) in groups.iter().enumerate() {
        list = list.child(render_sidebar_project_group(
            group,
            workspaces,
            active_workspace_index,
            shortcut_labels,
            pane_activity,
            group_index + 1 == group_count,
            cx,
        ));
    }
    list
}

fn render_sidebar_footer(cx: &mut Context<OcttyApp>) -> gpui::Div {
    div()
        .border_t_1()
        .border_color(rgb(0x4d545f))
        .p_2()
        .flex()
        .flex_col()
        .gap_2()
        .child(sidebar_add_repository_button().on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _, window, cx| {
                this.add_project_root(&AddProjectRoot, window, cx);
            }),
        ))
        .child(
            div()
                .grid()
                .grid_cols(3)
                .gap_2()
                .child(
                    sidebar_pane_button(IconName::SquareTerminal, "Shell").on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.add_pane(PaneType::Shell, cx);
                        }),
                    ),
                )
                .child(sidebar_pane_button(IconName::Replace, "Diff").on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.add_pane(PaneType::Diff, cx);
                    }),
                ))
                .child(sidebar_pane_button(IconName::File, "Note").on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.add_pane(PaneType::Note, cx);
                    }),
                )),
        )
}

fn sidebar_add_repository_button() -> gpui::Div {
    sidebar_control_button()
        .w_full()
        .justify_center()
        .gap_2()
        .child(Icon::new(IconName::FolderOpen).size(px(14.0)))
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::BOLD)
                .child("Add repository"),
        )
}

fn sidebar_pane_button(icon: IconName, label: &'static str) -> gpui::Div {
    sidebar_control_button()
        .justify_center()
        .gap_1()
        .child(Icon::new(icon).size(px(13.0)))
        .child(div().text_xs().child(label))
}

fn sidebar_control_button() -> gpui::Div {
    div()
        .h(px(28.0))
        .px_2()
        .flex()
        .items_center()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x4d545f))
        .bg(rgb(0x282c34))
        .text_color(rgb(0xd7dce4))
        .cursor_pointer()
}

fn render_error_toast(id: u64, message: SharedString, cx: &mut Context<OcttyApp>) -> gpui::Div {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(0xb94a52))
        .bg(rgb(0x35191d))
        .text_color(rgb(0xf7d4d7))
        .shadow_lg()
        .px_3()
        .py_2()
        .child(
            div()
                .pt(px(2.0))
                .text_color(rgb(0xff777f))
                .child(Icon::new(IconName::TriangleAlert).size(px(14.0))),
        )
        .child(div().flex_1().text_sm().child(message))
        .child(
            div()
                .p_1()
                .rounded_sm()
                .cursor_pointer()
                .text_color(rgb(0xf7d4d7))
                .hover(|this| this.bg(rgb(0x52252b)))
                .child(Icon::new(IconName::Close).size(px(12.0)))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _window, cx| {
                        this.dismiss_toast(id, cx);
                    }),
                ),
        )
}

fn render_sidebar_project_group(
    group: &SidebarWorkspaceGroup,
    workspaces: &[WorkspaceSummary],
    active_workspace_index: Option<usize>,
    shortcut_labels: &HashMap<String, String>,
    pane_activity: &HashMap<(String, String), PaneActivity>,
    is_last: bool,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let (title, root_id) = match &group.root {
        Some(root) => (root.display_name.clone(), Some(root.id.clone())),
        None => ("Other repos".to_owned(), None),
    };
    let mut section = div().pb_3();
    if !is_last {
        section = section.border_b_1().border_color(rgb(0x4d545f));
    }

    let mut title_row = div()
        .text_lg()
        .font_weight(gpui::FontWeight::BOLD)
        .truncate();
    title_row = title_row.child(title);
    let mut header = div().py_2().child(title_row);
    if let Some(root_id) = root_id.clone() {
        let entries = OcttyApp::project_root_menu_entries(&root_id);
        header = header.on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
                this.show_sidebar_menu(entries.clone(), event.position, window, cx);
            }),
        );
    }

    let mut workspace_list = div().flex().flex_col().gap_1();
    if group.workspace_indices.is_empty() {
        workspace_list = workspace_list.child(
            div()
                .py_2()
                .text_xs()
                .text_color(rgb(0x98a1ad))
                .child("No workspaces discovered."),
        );
    }

    for index in &group.workspace_indices {
        let workspace = &workspaces[*index];
        workspace_list = workspace_list.child(render_sidebar_workspace_row(
            *index,
            workspace,
            active_workspace_index == Some(*index),
            shortcut_labels.get(&workspace.id).map(String::as_str),
            workspace_activity_state(workspace, pane_activity),
            cx,
        ));
    }

    section = section.child(header);
    section.child(workspace_list)
}

fn render_sidebar_workspace_row(
    index: usize,
    workspace: &WorkspaceSummary,
    active: bool,
    shortcut_label: Option<&str>,
    activity_state: ActivityState,
    cx: &mut Context<OcttyApp>,
) -> impl IntoElement {
    let bookmark_label = workspace_bookmark_label(workspace);
    let has_meta_row = bookmark_label.is_some() || shortcut_label.is_some();
    let missing_path = !has_recorded_workspace_path(&workspace.workspace_path);
    let has_unread_notes = workspace.status.unread_notes > 0;

    let mut meta_row = div()
        .mt_1()
        .flex()
        .gap_2()
        .text_xs()
        .text_color(rgb(0x98a1ad));
    if let Some(bookmark_label) = bookmark_label {
        meta_row = meta_row.child(div().truncate().child(bookmark_label));
    }
    if let Some(shortcut_label) = shortcut_label {
        meta_row = meta_row.child(format!("<{shortcut_label}>"));
    }

    let mut badge_row = div().mt_1().flex().gap_1();
    if missing_path {
        badge_row = badge_row.child(Tag::warning().outline().xsmall().child("missing path"));
    }
    if has_unread_notes {
        badge_row = badge_row.child(
            Tag::secondary()
                .outline()
                .xsmall()
                .child(format!("{} note", workspace.status.unread_notes)),
        );
    }
    let workspace_id = workspace.id.clone();
    let can_forget = workspace.workspace_name != "default";
    let can_delete = can_forget && has_recorded_workspace_path(&workspace.workspace_path);
    let mut row = div().relative().py_2().on_mouse_up(
        MouseButton::Left,
        cx.listener(move |this, _, window, cx| {
            this.open_workspace(&OpenWorkspaceShortcut { index }, window, cx);
        }),
    );
    row = row.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |this, event: &MouseDownEvent, window, cx| {
            window.prevent_default();
            cx.stop_propagation();
            this.focus_handle.focus(window);
            if this.active_workspace_index != Some(index) {
                this.open_workspace(&OpenWorkspaceShortcut { index }, window, cx);
            }
            let entries = OcttyApp::workspace_menu_entries(&workspace_id, !can_forget, !can_delete);
            this.show_sidebar_menu(entries, event.position, window, cx);
        }),
    );
    if active {
        row = row.child(
            div()
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .left(-px(10.0))
                .right(-px(10.0))
                .border_1()
                .border_color(rgb(0x4d545f))
                .rounded_md()
                .bg(rgb(0x3c424d)),
        );
    }
    let mut workspace_name = div()
        .flex_1()
        .text_sm()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(if active { rgb(0x4e86d8) } else { rgb(0xd7dce4) })
        .truncate();
    workspace_name = workspace_name.child(workspace.display_name_or_workspace_name().to_owned());

    row = row.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap_2()
            .child(render_workspace_activity_icon(activity_state))
            .child(workspace_name)
            .child(render_workspace_status_tag(
                &workspace.status.workspace_state,
                workspace.status.has_working_copy_changes,
            )),
    );
    if has_meta_row {
        row = row.child(meta_row.relative());
    }
    if missing_path || has_unread_notes {
        row = row.child(badge_row.relative());
    }
    row
}

fn render_workspace_activity_icon(activity_state: ActivityState) -> gpui::Div {
    let base = div()
        .relative()
        .flex_none()
        .w(px(9.0))
        .h(px(9.0))
        .rounded_full();
    match activity_state {
        ActivityState::Active => base.border_2().border_color(rgb(0x5f7bff)).child(
            div()
                .absolute()
                .top(-px(1.0))
                .right(-px(1.0))
                .w(px(3.0))
                .h(px(3.0))
                .rounded_full()
                .bg(rgb(0x5f7bff)),
        ),
        ActivityState::IdleUnseen => base.bg(rgb(0x4a7cff)),
        ActivityState::IdleSeen => base.bg(rgb(0x6b7280)),
    }
}

fn render_workspace_status_tag(state: &WorkspaceState, changed: bool) -> Tag {
    let tag = match state {
        WorkspaceState::Published => Tag::success(),
        WorkspaceState::MergedLocal => Tag::warning(),
        WorkspaceState::Draft => Tag::info(),
        WorkspaceState::Conflicted => Tag::danger(),
        WorkspaceState::Unknown => Tag::secondary(),
    };
    let label = if changed {
        format!("{}*", workspace_status_label(state))
    } else {
        workspace_status_label(state).to_owned()
    };

    tag.outline().xsmall().child(label)
}

fn workspace_bookmark_label(workspace: &WorkspaceSummary) -> Option<String> {
    if workspace.status.bookmarks.is_empty() {
        return None;
    }

    let mut label = workspace.status.bookmarks.join(", ");
    if workspace.status.bookmark_relation == WorkspaceBookmarkRelation::Above {
        label.push_str(" (+)");
    }
    Some(label)
}

impl Render for SidebarMenuView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("octty-sidebar-menu")
            .key_context("OcttySidebarMenu")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .w(px(190.0))
            .p_1()
            .rounded_md()
            .border_1()
            .border_color(rgb(0x4d545f))
            .bg(rgb(0x23272f))
            .shadow_lg()
            .text_sm()
            .text_color(rgb(0xd7dce4))
            .children(
                self.entries
                    .iter()
                    .enumerate()
                    .filter(|(index, entry)| {
                        !matches!(entry, SidebarMenuEntry::Separator)
                            || self
                                .entries
                                .get(index + 1)
                                .is_some_and(|next| !matches!(next, SidebarMenuEntry::Separator))
                    })
                    .map(|(index, entry)| match entry {
                        SidebarMenuEntry::Separator => div()
                            .id(("sidebar-menu-separator", index))
                            .my_1()
                            .h(px(1.0))
                            .bg(rgb(0x4d545f))
                            .into_any_element(),
                        SidebarMenuEntry::Item {
                            label, disabled, ..
                        } => {
                            let selected = self.selected_index == Some(index);
                            div()
                                .id(("sidebar-menu-item", index))
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .when(selected && !disabled, |this| {
                                    this.bg(rgb(0x3c424d)).text_color(rgb(0xffffff))
                                })
                                .when(*disabled, |this| this.text_color(rgb(0x707987)))
                                .when(!disabled, |this| {
                                    this.cursor_pointer()
                                        .on_hover(cx.listener(move |this, hovered, _, cx| {
                                            if *hovered {
                                                this.selected_index = Some(index);
                                                cx.notify();
                                            }
                                        }))
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(move |this, _, window, cx| {
                                                window.prevent_default();
                                                cx.stop_propagation();
                                                this.execute_index(index, window, cx);
                                            }),
                                        )
                                })
                                .child(label.clone())
                                .into_any_element()
                        }
                    }),
            )
    }
}

fn rename_dialog_button(label: &'static str) -> gpui::Div {
    div()
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x4d545f))
        .bg(rgb(0x23272f))
        .text_sm()
        .cursor_pointer()
        .child(label)
}

fn rename_dialog_primary_button(label: &'static str) -> gpui::Div {
    rename_dialog_button(label)
        .border_color(rgb(0x4e86d8))
        .bg(rgb(0x2f5f9f))
        .text_color(rgb(0xffffff))
}

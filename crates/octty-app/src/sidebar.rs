use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarRenameTarget {
    ProjectRoot(String),
    Workspace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarMenuAction {
    CreateWorkspaceForRoot(String),
    RenameProjectRoot(String),
    RemoveProjectRoot(String),
    RenameWorkspace(String),
    ForgetWorkspace(String),
    DeleteAndForgetWorkspace(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SidebarMenuEntry {
    Item {
        label: SharedString,
        action: SidebarMenuAction,
        disabled: bool,
    },
    Separator,
}

impl SidebarMenuEntry {
    pub(crate) fn item(label: impl Into<SharedString>, action: SidebarMenuAction) -> Self {
        Self::item_with_disabled(label, action, false)
    }

    pub(crate) fn item_with_disabled(
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

    pub(crate) fn is_enabled_item(&self) -> bool {
        matches!(
            self,
            Self::Item {
                disabled: false,
                ..
            }
        )
    }

    pub(crate) fn action(&self) -> Option<SidebarMenuAction> {
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
pub(crate) struct SidebarMenuOverlay {
    pub(crate) position: Point<Pixels>,
    pub(crate) menu: Entity<SidebarMenuView>,
}

pub(crate) struct SidebarRenameDialog {
    pub(crate) target: SidebarRenameTarget,
    pub(crate) title: SharedString,
    pub(crate) input: Entity<InputState>,
}

#[derive(Clone)]
pub(crate) struct InnerSessionResumeDialog {
    pub(crate) shell_type: ShellTypeConfig,
    pub(crate) loading: bool,
    pub(crate) sessions: Vec<CodexSessionInfo>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) error: Option<SharedString>,
}

#[derive(Clone)]
pub(crate) struct AppToast {
    pub(crate) id: u64,
    pub(crate) message: SharedString,
}

pub(crate) struct SidebarMenuView {
    pub(crate) app: Entity<OcttyApp>,
    pub(crate) focus_handle: FocusHandle,
    pub(crate) entries: Vec<SidebarMenuEntry>,
    pub(crate) selected_index: Option<usize>,
}

impl SidebarMenuView {
    pub(crate) fn new(
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

    pub(crate) fn enabled_indices(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| entry.is_enabled_item().then_some(index))
            .collect()
    }

    pub(crate) fn select_up(
        &mut self,
        _: &SidebarMenuSelectUp,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn select_down(
        &mut self,
        _: &SidebarMenuSelectDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(crate) fn confirm(
        &mut self,
        _: &SidebarMenuConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.selected_index else {
            return;
        };
        self.execute_index(index, window, cx);
    }

    pub(crate) fn cancel(&mut self, _: &SidebarMenuCancel, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.app.update(cx, |app, cx| app.dismiss_sidebar_menu(cx));
    }

    pub(crate) fn execute_index(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.entries.get(index).and_then(SidebarMenuEntry::action) else {
            return;
        };
        let _ = self.app.update(cx, |app, cx| {
            app.execute_sidebar_menu_action(action, window, cx);
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SidebarWorkspaceGroup {
    pub(crate) root: Option<ProjectRootRecord>,
    pub(crate) workspace_indices: Vec<usize>,
}

pub(crate) fn sidebar_workspace_groups(
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

pub(crate) fn render_workspace_sidebar(
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

pub(crate) fn render_sidebar_footer(
    shell_types: &[ShellTypeConfig],
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let mut pane_buttons = div().grid().grid_cols(3).gap_2();
    for shell_type in shell_types {
        let shell_type_name = shell_type.name.clone();
        if shell_type.session_handler == InnerSessionHandler::None {
            pane_buttons = pane_buttons.child(
                sidebar_pane_button(IconName::SquareTerminal, shell_type.name.clone()).on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.add_shell_type_pane(&shell_type_name, cx);
                    }),
                ),
            );
        } else {
            let resume_shell_type_name = shell_type.name.clone();
            pane_buttons = pane_buttons.child(
                sidebar_split_pane_button(IconName::SquareTerminal, shell_type.name.clone())
                    .child(
                        sidebar_split_pane_button_main(
                            IconName::SquareTerminal,
                            shell_type.name.clone(),
                        )
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _, _, cx| {
                                this.add_shell_type_pane(&shell_type_name, cx);
                            }),
                        ),
                    )
                    .child(sidebar_split_pane_button_more().on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _, _, cx| {
                            this.open_inner_session_resume_dialog(&resume_shell_type_name, cx);
                        }),
                    )),
            );
        }
    }
    pane_buttons = pane_buttons
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
        ));

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
        .child(pane_buttons)
}

pub(crate) fn sidebar_add_repository_button() -> gpui::Div {
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

pub(crate) fn render_inner_session_resume_dialog(
    dialog: InnerSessionResumeDialog,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let title = format!("Resume {}", dialog.shell_type.name);
    let mut list = div()
        .mt_3()
        .h(px(320.0))
        .overflow_y_scrollbar()
        .flex()
        .flex_col()
        .gap_1();
    if dialog.loading {
        list = list.child(
            div()
                .py_3()
                .text_sm()
                .text_color(rgb(0x98a1ad))
                .child("Loading Codex sessions..."),
        );
    } else if let Some(error) = dialog.error.clone() {
        list = list.child(
            div()
                .py_3()
                .text_sm()
                .text_color(rgb(0xf7d4d7))
                .child(error),
        );
    } else if dialog.sessions.is_empty() {
        list = list.child(
            div()
                .py_3()
                .text_sm()
                .text_color(rgb(0x98a1ad))
                .child("No Codex sessions found."),
        );
    } else {
        for (index, session) in dialog.sessions.iter().enumerate() {
            list = list.child(render_inner_session_resume_row(
                index,
                session,
                dialog.selected_index == Some(index),
                cx,
            ));
        }
    }

    div()
        .w(px(520.0))
        .p_3()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x4d545f))
        .bg(rgb(0x23272f))
        .shadow_lg()
        .text_color(rgb(0xd7dce4))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(rgb(0x98a1ad))
                .child("Choose a Codex session to resume in a new pane."),
        )
        .child(list)
        .child(div().mt_3().flex().justify_end().gap_2().child(
            rename_dialog_button("Cancel").on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _window, cx| {
                    this.dismiss_inner_session_resume_dialog(cx);
                }),
            ),
        ))
}

fn render_inner_session_resume_row(
    index: usize,
    session: &CodexSessionInfo,
    selected: bool,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
    let mut row = div()
        .px_2()
        .py_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x3c434d))
        .cursor_pointer()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .truncate()
                .child(session.description.clone()),
        )
        .child(
            div()
                .mt_1()
                .text_xs()
                .text_color(rgb(0x98a1ad))
                .truncate()
                .child(format!(
                    "{}  {}",
                    session.timestamp, session.inner_session_id
                )),
        )
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.resume_inner_session(index, cx);
            }),
        );
    if selected {
        row = row.bg(rgb(0x384150)).border_color(rgb(0x8793a4));
    } else {
        row = row.bg(rgb(0x282c34));
    }
    row
}

pub(crate) fn sidebar_pane_button(icon: IconName, label: impl Into<SharedString>) -> gpui::Div {
    let label = label.into();
    sidebar_control_button()
        .justify_center()
        .gap_1()
        .child(Icon::new(icon).size(px(13.0)))
        .child(div().text_xs().child(label))
}

pub(crate) fn sidebar_split_pane_button(
    _icon: IconName,
    _label: impl Into<SharedString>,
) -> gpui::Div {
    div().h(px(28.0)).flex().items_center().gap_1()
}

pub(crate) fn sidebar_split_pane_button_main(
    icon: IconName,
    label: impl Into<SharedString>,
) -> gpui::Div {
    sidebar_control_button()
        .flex_1()
        .justify_center()
        .gap_1()
        .child(Icon::new(icon).size(px(13.0)))
        .child(div().text_xs().child(label.into()))
}

pub(crate) fn sidebar_split_pane_button_more() -> gpui::Div {
    sidebar_control_button()
        .w(px(28.0))
        .justify_center()
        .child(div().text_xs().child("..."))
}

pub(crate) fn sidebar_control_button() -> gpui::Div {
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

pub(crate) fn render_error_toast(
    id: u64,
    message: SharedString,
    cx: &mut Context<OcttyApp>,
) -> gpui::Div {
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

pub(crate) fn render_sidebar_project_group(
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
            workspace_activity_indicator(workspace, pane_activity),
            cx,
        ));
    }

    section = section.child(header);
    section.child(workspace_list)
}

pub(crate) fn render_sidebar_workspace_row(
    index: usize,
    workspace: &WorkspaceSummary,
    active: bool,
    shortcut_label: Option<&str>,
    activity_indicator: WorkspaceActivityIndicator,
    cx: &mut Context<OcttyApp>,
) -> impl IntoElement {
    let bookmark_label = workspace_bookmark_label(workspace);
    let has_conflicts = workspace.status.has_conflicts;
    let has_meta_row = bookmark_label.is_some() || shortcut_label.is_some() || has_conflicts;
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
    if has_conflicts {
        meta_row = meta_row.child(workspace_status_tag(
            Tag::danger(),
            "Conflict",
            false,
            Some("This workspace has unresolved jj conflicts."),
        ));
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
            .child(render_workspace_activity_icon(activity_indicator))
            .child(workspace_name)
            .children(render_workspace_status_tags(&workspace.status)),
    );
    if has_meta_row {
        row = row.child(meta_row.relative());
    }
    if missing_path || has_unread_notes {
        row = row.child(badge_row.relative());
    }
    row
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceActivityMarker {
    AttentionActive,
    AttentionIdle,
    Active,
    Idle,
}

pub(crate) fn workspace_activity_marker(
    activity_indicator: WorkspaceActivityIndicator,
) -> WorkspaceActivityMarker {
    let has_recent_activity = activity_indicator.activity_state == ActivityState::Active;
    match (activity_indicator.needs_attention, has_recent_activity) {
        (true, true) => WorkspaceActivityMarker::AttentionActive,
        (true, false) => WorkspaceActivityMarker::AttentionIdle,
        (false, true) => WorkspaceActivityMarker::Active,
        (false, false) => WorkspaceActivityMarker::Idle,
    }
}

pub(crate) fn render_workspace_activity_icon(
    activity_indicator: WorkspaceActivityIndicator,
) -> gpui::Div {
    let base = div()
        .relative()
        .flex_none()
        .w(px(9.0))
        .h(px(9.0))
        .rounded_full();
    match workspace_activity_marker(activity_indicator) {
        WorkspaceActivityMarker::AttentionActive => {
            base.child(render_workspace_activity_spinner(rgb(0xe5484d).into()))
        }
        WorkspaceActivityMarker::AttentionIdle => base.bg(rgb(0xe5484d)),
        WorkspaceActivityMarker::Active => {
            base.child(render_workspace_activity_spinner(rgb(0x5f7bff).into()))
        }
        WorkspaceActivityMarker::Idle => base.bg(rgb(0x6b7280)),
    }
}

fn render_workspace_activity_spinner(color: Hsla) -> impl IntoElement {
    workspace_activity_spinner_canvas(0.0, color).with_animation(
        "workspace-activity-spinner",
        Animation::new(Duration::from_millis(750)).repeat(),
        move |_, delta| workspace_activity_spinner_canvas(delta, color),
    )
}

fn workspace_activity_spinner_canvas(phase: f32, color: Hsla) -> impl IntoElement {
    const DOTS: usize = 8;
    const DOT_SIZE: f32 = 1.8;
    const RADIUS: f32 = 3.0;

    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center_x =
                bounds.origin.x.as_f64() as f32 + bounds.size.width.as_f64() as f32 / 2.0;
            let center_y =
                bounds.origin.y.as_f64() as f32 + bounds.size.height.as_f64() as f32 / 2.0;
            let head = phase * DOTS as f32;

            for dot in 0..DOTS {
                let angle = (dot as f32 / DOTS as f32) * std::f32::consts::TAU;
                let x = center_x + angle.cos() * RADIUS - DOT_SIZE / 2.0;
                let y = center_y + angle.sin() * RADIUS - DOT_SIZE / 2.0;
                let age = (head - dot as f32).rem_euclid(DOTS as f32);
                let opacity = 1.0 - (age / DOTS as f32) * 0.8;

                window.paint_quad(
                    fill(
                        Bounds {
                            origin: point(px(x), px(y)),
                            size: size(px(DOT_SIZE), px(DOT_SIZE)),
                        },
                        color.opacity(opacity),
                    )
                    .corner_radii(px(DOT_SIZE / 2.0)),
                );
            }
        },
    )
    .size_full()
}

pub(crate) fn render_workspace_status_tags(status: &WorkspaceStatus) -> Vec<AnyElement> {
    if status.workspace_state == WorkspaceState::Unknown {
        return vec![workspace_status_tag(
            Tag::secondary(),
            "Unknown",
            status.has_working_copy_changes,
            Some("Workspace status is unavailable."),
        )];
    }

    let mut tags = Vec::new();
    let dirty_on_relation = status.has_working_copy_changes && tags.is_empty();
    match status.primary_relation {
        BaselineRelationTarget::Local => {
            if let Some(relation) = &status.local_relation {
                tags.push(relation_status_tag(relation, dirty_on_relation));
            }
        }
        BaselineRelationTarget::Remote => {
            if let Some(relation) = &status.remote_relation {
                tags.push(relation_status_tag(relation, dirty_on_relation));
            }
        }
        BaselineRelationTarget::None => {}
    };

    if status.primary_relation != BaselineRelationTarget::Local
        && let Some(relation) = &status.local_relation
        && relation_has_changes(relation)
    {
        tags.push(relation_status_tag(relation, false));
    }
    if status.primary_relation != BaselineRelationTarget::Remote
        && let Some(relation) = &status.remote_relation
        && relation_has_changes(relation)
    {
        tags.push(relation_status_tag(relation, false));
    }

    if tags.is_empty() {
        tags.push(workspace_status_tag(
            Tag::secondary(),
            "Unknown",
            status.has_working_copy_changes,
            Some("Workspace status is unavailable."),
        ));
    }

    tags
}

fn relation_status_tag(relation: &BaselineRelation, changed: bool) -> AnyElement {
    let tag = match relation.state() {
        BaselineRelationState::Same => Tag::success(),
        BaselineRelationState::Ahead => Tag::info(),
        BaselineRelationState::Behind => Tag::warning(),
        BaselineRelationState::Diverged => Tag::warning(),
    };
    workspace_status_tag(
        tag,
        format_relation(relation),
        changed,
        Some(format_relation_tooltip(relation)),
    )
}

fn relation_has_changes(relation: &BaselineRelation) -> bool {
    relation.ahead_count > 0 || relation.behind_count > 0
}

fn workspace_status_tag(
    tag: Tag,
    label: impl Into<String>,
    changed: bool,
    tooltip: Option<impl Into<String>>,
) -> AnyElement {
    let mut label = label.into();
    if changed {
        label.push('*');
    }
    let element_id_label = label.clone();
    let tag = tag
        .xsmall()
        .px_1()
        .py_0()
        .text_size(px(10.))
        .text_color(rgb(0xffffff))
        .child(label);

    if let Some(tooltip) = tooltip {
        let tooltip = tooltip.into();
        div()
            .id(SharedString::from(element_id_label))
            .child(tag)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .into_any_element()
    } else {
        tag.into_any_element()
    }
}

fn format_relation(relation: &BaselineRelation) -> String {
    let target = relation.target_name.as_str();
    match (relation.ahead_count, relation.behind_count) {
        (0, 0) => format!("{target} 0"),
        (ahead, 0) => format!("{target} +{ahead}"),
        (0, behind) => format!("{target} -{behind}"),
        (ahead, behind) => format!("{target} +{ahead}/-{behind}"),
    }
}

fn format_relation_tooltip(relation: &BaselineRelation) -> String {
    let target = relation
        .detail_name
        .as_deref()
        .unwrap_or(relation.target_name.as_str());
    let ahead = relation.ahead_count;
    let behind = relation.behind_count;
    match (ahead, behind) {
        (0, 0) => {
            format!("No non-empty commit difference between this workspace and {target}.")
        }
        (ahead, 0) => format!(
            "+{ahead}: {} in this workspace and not in {target}.",
            commit_word(ahead)
        ),
        (0, behind) => format!(
            "-{behind}: {} in {target} and not in this workspace.",
            commit_word(behind)
        ),
        (ahead, behind) => format!(
            "+{ahead}: {} in this workspace and not in {target}. -{behind}: {} in {target} and not in this workspace.",
            commit_word(ahead),
            commit_word(behind),
        ),
    }
}

fn commit_word(count: i64) -> &'static str {
    if count == 1 {
        "commit is"
    } else {
        "commits are"
    }
}

pub(crate) fn workspace_bookmark_label(workspace: &WorkspaceSummary) -> Option<String> {
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

pub(crate) fn rename_dialog_button(label: &'static str) -> gpui::Div {
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

pub(crate) fn rename_dialog_primary_button(label: &'static str) -> gpui::Div {
    rename_dialog_button(label)
        .border_color(rgb(0x4e86d8))
        .bg(rgb(0x2f5f9f))
        .text_color(rgb(0xffffff))
}

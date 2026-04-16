use super::*;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct OpenWorkspaceShortcut {
    pub(crate) index: usize,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct NavigateWorkspace {
    pub(crate) direction: WorkspaceNavigationDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct AddShellPane {
    pub(crate) shell_type: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct AddDiffPane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct AddNotePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct PasteTerminalClipboard;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct CopyTerminalSelection;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct CutTerminalSelection;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct ForwardTerminalTab {
    pub(crate) shift: bool,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct NavigatePane {
    pub(crate) direction: PaneNavigationDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct CloseActivePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct ResizeFocusedColumn {
    pub(crate) direction: ColumnResizeDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct AddProjectRoot;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct CreateWorkspaceForRoot {
    pub(crate) root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct RenameProjectRoot {
    pub(crate) root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct RemoveProjectRoot {
    pub(crate) root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct RenameWorkspace {
    pub(crate) workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct ForgetWorkspace {
    pub(crate) workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct DeleteAndForgetWorkspace {
    pub(crate) workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct SidebarMenuSelectUp;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct SidebarMenuSelectDown;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct SidebarMenuConfirm;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
pub(crate) struct SidebarMenuCancel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneNavigationDirection {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkspaceNavigationDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColumnResizeDirection {
    Slimmer,
    Wider,
}

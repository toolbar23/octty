#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct OpenWorkspaceShortcut {
    index: usize,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct NavigateWorkspace {
    direction: WorkspaceNavigationDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddShellPane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddDiffPane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddNotePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct PasteTerminalClipboard;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct CopyTerminalSelection;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct CutTerminalSelection;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct NavigatePane {
    direction: PaneNavigationDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct CloseActivePane;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct ResizeFocusedColumn {
    direction: ColumnResizeDirection,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct AddProjectRoot;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct CreateWorkspaceForRoot {
    root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct RenameProjectRoot {
    root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct RemoveProjectRoot {
    root_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct RenameWorkspace {
    workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct ForgetWorkspace {
    workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct DeleteAndForgetWorkspace {
    workspace_id: String,
}

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct SidebarMenuSelectUp;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct SidebarMenuSelectDown;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct SidebarMenuConfirm;

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = octty, no_json)]
struct SidebarMenuCancel;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaneNavigationDirection {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorkspaceNavigationDirection {
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColumnResizeDirection {
    Slimmer,
    Wider,
}

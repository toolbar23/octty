use super::*;

pub(crate) fn set_workspace_menu(cx: &mut App, workspaces: &[WorkspaceSummary]) {
    cx.set_menus(vec![Menu {
        name: "Workspaces".into(),
        items: workspace_menu_items(workspaces),
    }]);
}

pub(crate) fn workspace_menu_items(workspaces: &[WorkspaceSummary]) -> Vec<MenuItem> {
    workspace_shortcut_targets(workspaces)
        .into_iter()
        .enumerate()
        .map(|(index, target)| {
            let workspace = &workspaces[index];
            let name = format!(
                "{} <{}>",
                workspace.display_name_or_workspace_name(),
                target.label
            );
            MenuItem::action(name, OpenWorkspaceShortcut { index })
        })
        .collect()
}

pub(crate) fn workspace_key_bindings(shell_types: &[ShellTypeConfig]) -> Vec<KeyBinding> {
    let mut bindings = vec![
        KeyBinding::new("ctrl-shift-1", OpenWorkspaceShortcut { index: 0 }, None),
        KeyBinding::new("ctrl-shift-2", OpenWorkspaceShortcut { index: 1 }, None),
        KeyBinding::new("ctrl-shift-3", OpenWorkspaceShortcut { index: 2 }, None),
        KeyBinding::new("ctrl-shift-4", OpenWorkspaceShortcut { index: 3 }, None),
        KeyBinding::new("ctrl-shift-5", OpenWorkspaceShortcut { index: 4 }, None),
        KeyBinding::new("ctrl-shift-6", OpenWorkspaceShortcut { index: 5 }, None),
        KeyBinding::new("ctrl-shift-7", OpenWorkspaceShortcut { index: 6 }, None),
        KeyBinding::new("ctrl-shift-8", OpenWorkspaceShortcut { index: 7 }, None),
        KeyBinding::new("ctrl-shift-9", OpenWorkspaceShortcut { index: 8 }, None),
        KeyBinding::new("ctrl-shift-0", OpenWorkspaceShortcut { index: 9 }, None),
        KeyBinding::new("ctrl-shift-c", CopyTerminalSelection, None),
        KeyBinding::new("ctrl-shift-x", CutTerminalSelection, None),
        KeyBinding::new("ctrl-shift-v", PasteTerminalClipboard, None),
        KeyBinding::new("super-c", CopyTerminalSelection, None),
        KeyBinding::new("super-x", CutTerminalSelection, None),
        KeyBinding::new("super-v", PasteTerminalClipboard, None),
        KeyBinding::new("super-p", PasteTerminalClipboard, None),
        KeyBinding::new("cmd-v", PasteTerminalClipboard, None),
        KeyBinding::new("tab", ForwardTerminalTab { shift: false }, Some("OcttyApp")),
        KeyBinding::new(
            "shift-tab",
            ForwardTerminalTab { shift: true },
            Some("OcttyApp"),
        ),
        KeyBinding::new(
            "ctrl-shift-left",
            NavigatePane {
                direction: PaneNavigationDirection::Left,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-right",
            NavigatePane {
                direction: PaneNavigationDirection::Right,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-up",
            NavigateWorkspace {
                direction: WorkspaceNavigationDirection::Previous,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-shift-down",
            NavigateWorkspace {
                direction: WorkspaceNavigationDirection::Next,
            },
            None,
        ),
        KeyBinding::new("ctrl-shift-w", CloseActivePane, None),
        KeyBinding::new("up", SidebarMenuSelectUp, Some("OcttySidebarMenu")),
        KeyBinding::new("down", SidebarMenuSelectDown, Some("OcttySidebarMenu")),
        KeyBinding::new("enter", SidebarMenuConfirm, Some("OcttySidebarMenu")),
        KeyBinding::new("escape", SidebarMenuCancel, Some("OcttySidebarMenu")),
        KeyBinding::new(
            "ctrl-alt-left",
            ResizeFocusedColumn {
                direction: ColumnResizeDirection::Slimmer,
            },
            None,
        ),
        KeyBinding::new(
            "ctrl-alt-right",
            ResizeFocusedColumn {
                direction: ColumnResizeDirection::Wider,
            },
            None,
        ),
    ];

    bindings.extend(shell_types.iter().filter_map(|shell_type| {
        let shortcut = shell_type.shortcut.trim();
        (!shortcut.is_empty()).then(|| {
            KeyBinding::new(
                shortcut,
                AddShellPane {
                    shell_type: shell_type.name.clone(),
                },
                None,
            )
        })
    }));
    bindings
}

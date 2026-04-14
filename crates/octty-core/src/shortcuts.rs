use crate::types::WorkspaceSummary;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceShortcutTarget {
    pub workspace_id: String,
    pub label: String,
}

pub fn workspace_shortcut_targets(workspaces: &[WorkspaceSummary]) -> Vec<WorkspaceShortcutTarget> {
    workspaces
        .iter()
        .take(10)
        .enumerate()
        .map(|(index, workspace)| WorkspaceShortcutTarget {
            workspace_id: workspace.id.clone(),
            label: format!("Ctrl+Shift+{}", shortcut_digit(index)),
        })
        .collect()
}

fn shortcut_digit(index: usize) -> &'static str {
    match index {
        0 => "1",
        1 => "2",
        2 => "3",
        3 => "4",
        4 => "5",
        5 => "6",
        6 => "7",
        7 => "8",
        8 => "9",
        _ => "0",
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{WorkspaceStatus, WorkspaceSummary};

    use super::*;

    #[test]
    fn first_ten_workspaces_get_ctrl_shift_number_shortcuts() {
        let workspaces = (0..12)
            .map(|index| WorkspaceSummary {
                id: format!("workspace-{index}"),
                root_id: "root".to_owned(),
                root_path: "/repo".to_owned(),
                project_display_name: "repo".to_owned(),
                workspace_name: format!("w{index}"),
                display_name: format!("w{index}"),
                workspace_path: format!("/repo/w{index}"),
                status: WorkspaceStatus::default(),
                created_at: 0,
                updated_at: 0,
                last_opened_at: 0,
            })
            .collect::<Vec<_>>();

        let targets = workspace_shortcut_targets(&workspaces);

        assert_eq!(targets.len(), 10);
        assert_eq!(targets[0].label, "Ctrl+Shift+1");
        assert_eq!(targets[8].label, "Ctrl+Shift+9");
        assert_eq!(targets[9].label, "Ctrl+Shift+0");
    }
}

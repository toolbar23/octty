use std::path::Path;
use std::process::Stdio;

use octty_core::{
    ProjectRootRecord, WorkspaceState, WorkspaceStatus, WorkspaceSummary,
    encode_missing_workspace_path,
};
use thiserror::Error;
use tokio::process::Command;

const WORKSPACE_LIST_SEPARATOR: &str = "\u{1f}";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredWorkspace {
    pub workspace_name: String,
    pub target_change_id: String,
}

#[derive(Debug, Error)]
pub enum JjError {
    #[error("jj command failed: {0}")]
    Command(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn parse_workspace_list_output(output: &str) -> Vec<DiscoveredWorkspace> {
    output
        .lines()
        .filter_map(|line| {
            let (name, target) = line.split_once(WORKSPACE_LIST_SEPARATOR)?;
            let workspace_name = name.trim();
            if workspace_name.is_empty() {
                return None;
            }
            Some(DiscoveredWorkspace {
                workspace_name: workspace_name.to_owned(),
                target_change_id: target.trim().to_owned(),
            })
        })
        .collect()
}

pub fn is_stale_workspace_error(message: &str) -> bool {
    message.contains("working copy is stale") && message.contains("jj workspace update-stale")
}

pub async fn discover_workspaces(
    root: &ProjectRootRecord,
) -> Result<Vec<WorkspaceSummary>, JjError> {
    let template = format!("workspace ++ \"{WORKSPACE_LIST_SEPARATOR}\" ++ change_id ++ \"\\n\"");
    let output = run_jj(
        root.root_path.as_ref(),
        &["workspace", "list", "--template", template.as_str()],
    )
    .await?;
    let entries = parse_workspace_list_output(&output);
    let mut summaries = Vec::with_capacity(entries.len());
    for entry in entries {
        let workspace_path = workspace_path(&root.root_path, &entry.workspace_name)
            .await
            .unwrap_or_else(|_| encode_missing_workspace_path(&entry.workspace_name));
        summaries.push(WorkspaceSummary {
            id: stable_workspace_id(&root.root_path, &entry.workspace_name),
            root_id: root.id.clone(),
            root_path: root.root_path.clone(),
            project_display_name: root.display_name.clone(),
            workspace_name: entry.workspace_name.clone(),
            display_name: entry.workspace_name,
            workspace_path,
            status: WorkspaceStatus::default(),
            created_at: 0,
            updated_at: 0,
            last_opened_at: 0,
        });
    }
    Ok(summaries)
}

pub async fn read_workspace_status(
    workspace_path: impl AsRef<Path>,
) -> Result<WorkspaceStatus, JjError> {
    let workspace_path = workspace_path.as_ref();
    let diff = with_stale_retry(
        workspace_path,
        &["diff", "-r", "@", "--git", "--color=never"],
    )
    .await?;
    let conflicts = with_stale_retry(
        workspace_path,
        &["log", "-r", "conflicts() & @", "--no-graph"],
    )
    .await?;
    let has_working_copy_changes = !diff.trim().is_empty();
    let has_conflicts = !conflicts.trim().is_empty();
    let workspace_state = if has_conflicts {
        WorkspaceState::Conflicted
    } else if has_working_copy_changes {
        WorkspaceState::Draft
    } else {
        WorkspaceState::Published
    };

    Ok(WorkspaceStatus {
        workspace_state,
        has_working_copy_changes,
        has_conflicts,
        diff_text: diff,
        ..WorkspaceStatus::default()
    })
}

async fn workspace_path(root_path: &str, workspace_name: &str) -> Result<String, JjError> {
    let output = run_jj(
        root_path.as_ref(),
        &["workspace", "root", "--workspace", workspace_name],
    )
    .await?;
    Ok(output.trim().to_owned())
}

async fn with_stale_retry(workspace_path: &Path, args: &[&str]) -> Result<String, JjError> {
    match run_jj(workspace_path, args).await {
        Err(JjError::Command(message)) if is_stale_workspace_error(&message) => {
            run_jj(workspace_path, &["workspace", "update-stale"]).await?;
            run_jj(workspace_path, args).await
        }
        result => result,
    }
}

async fn run_jj(cwd: &Path, args: &[&str]) -> Result<String, JjError> {
    let output = Command::new("jj")
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .await?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(JjError::Command(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    }
}

fn stable_workspace_id(root_path: &str, workspace_name: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in root_path.bytes().chain([0]).chain(workspace_name.bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("workspace-{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jj_workspace_list_template_output() {
        let output =
            format!("default{WORKSPACE_LIST_SEPARATOR}abc\nfeature{WORKSPACE_LIST_SEPARATOR}def\n");
        let parsed = parse_workspace_list_output(&output);

        assert_eq!(
            parsed,
            vec![
                DiscoveredWorkspace {
                    workspace_name: "default".to_owned(),
                    target_change_id: "abc".to_owned()
                },
                DiscoveredWorkspace {
                    workspace_name: "feature".to_owned(),
                    target_change_id: "def".to_owned()
                }
            ]
        );
    }

    #[test]
    fn recognizes_stale_workspace_errors() {
        assert!(is_stale_workspace_error(
            "The working copy is stale. Hint: Run `jj workspace update-stale` to update it."
        ));
    }
}

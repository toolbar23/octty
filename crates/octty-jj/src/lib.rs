use std::path::{Path, PathBuf};
use std::process::Stdio;

use octty_core::{
    BaselineRelation, BaselineRelationTarget, ProjectRootRecord, WorkspaceBookmarkRelation,
    WorkspaceState, WorkspaceStatus, WorkspaceSummary, encode_missing_workspace_path,
    layout::now_ms,
};
use thiserror::Error;
use tokio::process::Command;

const WORKSPACE_LIST_SEPARATOR: &str = "\t";
pub const EFFECTIVE_WORKSPACE_REVSET: &str = "coalesce(@ ~ empty(), @-)";
pub const DISPLAY_BOOKMARK_REVSET: &str =
    "heads(first_ancestors(coalesce(@ ~ empty(), @-)) & bookmarks())";
pub const CONFLICTED_WORKSPACE_REVSET: &str = "coalesce(@ ~ empty(), @-) & conflicts()";
pub const DEFAULT_WORKSPACE_REVSET: &str = "present(default@)";
pub const LOCAL_AHEAD_REVSET: &str = "default@..@ ~ empty()";
pub const LOCAL_BEHIND_REVSET: &str = "@..default@ ~ empty()";

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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DiffStats {
    pub added_lines: i64,
    pub removed_lines: i64,
}

pub fn summarize_unified_diff(diff_text: &str) -> DiffStats {
    let mut stats = DiffStats::default();
    for line in diff_text.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            stats.added_lines += 1;
        } else if line.starts_with('-') {
            stats.removed_lines += 1;
        }
    }
    stats
}

pub fn parse_count(output: &str) -> i64 {
    output.trim().parse().unwrap_or(0)
}

pub fn parse_bookmarks(output: &str) -> Vec<String> {
    output
        .trim()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub fn classify_bookmark_relation(
    exact_bookmarks: &[String],
    display_bookmarks: &[String],
) -> WorkspaceBookmarkRelation {
    if !exact_bookmarks.is_empty() {
        WorkspaceBookmarkRelation::Exact
    } else if !display_bookmarks.is_empty() {
        WorkspaceBookmarkRelation::Above
    } else {
        WorkspaceBookmarkRelation::None
    }
}

pub fn classify_workspace_state(
    has_conflicts: bool,
    primary_relation: BaselineRelationTarget,
    relation: Option<&BaselineRelation>,
) -> WorkspaceState {
    if has_conflicts {
        WorkspaceState::Conflicted
    } else {
        match (primary_relation, relation) {
            (BaselineRelationTarget::Remote, Some(relation))
                if relation.ahead_count == 0 && relation.behind_count == 0 =>
            {
                WorkspaceState::Published
            }
            (BaselineRelationTarget::Local, Some(relation))
                if relation.ahead_count == 0 && relation.behind_count == 0 =>
            {
                WorkspaceState::MergedLocal
            }
            (BaselineRelationTarget::Remote | BaselineRelationTarget::Local, Some(_)) => {
                WorkspaceState::Draft
            }
            _ => WorkspaceState::Unknown,
        }
    }
}

pub async fn resolve_repo_root(input_path: impl AsRef<Path>) -> Result<PathBuf, JjError> {
    let input_path = input_path.as_ref();
    let root = with_stale_retry(input_path, &["root", "-R", path_str(input_path)?]).await?;
    Ok(tokio::fs::canonicalize(root.trim()).await?)
}

pub async fn discover_workspaces(
    root: &ProjectRootRecord,
) -> Result<Vec<WorkspaceSummary>, JjError> {
    let root_path = tokio::fs::canonicalize(&root.root_path).await?;
    let root_path = path_str(&root_path)?;
    let template =
        format!("name ++ \"{WORKSPACE_LIST_SEPARATOR}\" ++ target.change_id().short() ++ \"\\n\"");
    let output = run_jj(
        root_path.as_ref(),
        &[
            "workspace",
            "list",
            "-R",
            root_path,
            "--template",
            template.as_str(),
        ],
    )
    .await?;
    let entries = parse_workspace_list_output(&output);
    let current_change_id = current_workspace_change_id(root_path).await.ok();
    let current_workspace_path = current_workspace_path(root_path).await.ok();
    let mut summaries = Vec::with_capacity(entries.len());
    for entry in entries {
        let workspace_path = workspace_path(root_path, &entry.workspace_name)
            .await
            .or_else(|_| {
                if current_change_id.as_deref() == Some(entry.target_change_id.as_str()) {
                    current_workspace_path.clone().ok_or_else(|| {
                        JjError::Command("current workspace path unavailable".to_owned())
                    })
                } else {
                    Err(JjError::Command(format!(
                        "workspace has no recorded path: {}",
                        entry.workspace_name
                    )))
                }
            })
            .unwrap_or_else(|_| encode_missing_workspace_path(&entry.workspace_name));
        summaries.push(WorkspaceSummary {
            id: workspace_id_for(root_path, &entry.workspace_name),
            root_id: root.id.clone(),
            root_path: root_path.to_owned(),
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
    workspace_name: impl AsRef<str>,
) -> Result<WorkspaceStatus, JjError> {
    let workspace_path = workspace_path.as_ref();
    let workspace_name = workspace_name.as_ref();
    let exact_bookmark_output = with_stale_retry(
        workspace_path,
        &[
            "log",
            "-r",
            EFFECTIVE_WORKSPACE_REVSET,
            "-n",
            "1",
            "--no-graph",
            "-T",
            "bookmarks.map(|b| b.name()).join(\",\") ++ \"\\n\"",
        ],
    )
    .await?;
    let display_bookmark_output = with_stale_retry(
        workspace_path,
        &[
            "log",
            "-r",
            DISPLAY_BOOKMARK_REVSET,
            "-n",
            "1",
            "--no-graph",
            "-T",
            "bookmarks.map(|b| b.name()).join(\",\") ++ \"\\n\"",
        ],
    )
    .await?;
    let diff = with_stale_retry(
        workspace_path,
        &["diff", "-r", "@", "--git", "--color=never"],
    )
    .await?;
    let conflicted_count = count_revset(workspace_path, CONFLICTED_WORKSPACE_REVSET).await?;
    let exact_bookmarks = parse_bookmarks(&exact_bookmark_output);
    let display_bookmarks = parse_bookmarks(&display_bookmark_output);
    let bookmark_relation = classify_bookmark_relation(&exact_bookmarks, &display_bookmarks);
    let bookmarks = if exact_bookmarks.is_empty() {
        display_bookmarks
    } else {
        exact_bookmarks
    };
    let local_relation = local_relation(workspace_path).await?;
    let remote_relation = remote_relation(workspace_path, &bookmarks).await?;
    let primary_relation = primary_relation(workspace_name, &local_relation, &remote_relation);
    let primary_baseline_relation = match primary_relation {
        BaselineRelationTarget::Local => local_relation.as_ref(),
        BaselineRelationTarget::Remote => remote_relation.as_ref(),
        BaselineRelationTarget::None => None,
    };
    let has_conflicts = conflicted_count > 0;
    let has_working_copy_changes = !diff.trim().is_empty();
    let workspace_state = classify_workspace_state(
        has_conflicts,
        primary_relation.clone(),
        primary_baseline_relation,
    );

    Ok(WorkspaceStatus {
        workspace_state,
        has_working_copy_changes,
        has_conflicts,
        local_relation,
        remote_relation,
        primary_relation,
        bookmarks,
        bookmark_relation,
        recent_activity_at: now_ms(),
        diff_text: diff,
        ..WorkspaceStatus::default()
    })
}

async fn count_revset(workspace_path: &Path, revset: &str) -> Result<i64, JjError> {
    Ok(parse_count(
        &with_stale_retry(workspace_path, &["log", "-r", revset, "--count"]).await?,
    ))
}

async fn local_relation(workspace_path: &Path) -> Result<Option<BaselineRelation>, JjError> {
    if count_revset(workspace_path, DEFAULT_WORKSPACE_REVSET).await? == 0 {
        return Ok(None);
    }
    baseline_relation(
        workspace_path,
        "Default",
        Some("default@"),
        LOCAL_AHEAD_REVSET,
        LOCAL_BEHIND_REVSET,
    )
    .await
}

async fn remote_relation(
    workspace_path: &Path,
    bookmarks: &[String],
) -> Result<Option<BaselineRelation>, JjError> {
    for bookmark in bookmarks {
        let remote_revset = format!(
            "tracked_remote_bookmarks(exact:{})",
            revset_string_literal(bookmark)
        );
        if count_revset(workspace_path, &remote_revset).await? == 1 {
            let detail_name = remote_bookmark_detail(workspace_path, &remote_revset, bookmark)
                .await?
                .unwrap_or_else(|| {
                    format!(
                        "tracked_remote_bookmarks(exact:{})",
                        revset_string_literal(bookmark)
                    )
                });
            return baseline_relation(
                workspace_path,
                "Remote",
                Some(detail_name),
                &format!("{remote_revset}..@ ~ empty()"),
                &format!("@..{remote_revset} ~ empty()"),
            )
            .await;
        }
    }

    if count_revset(workspace_path, "present(trunk()) ~ root()").await? == 1 {
        return baseline_relation(
            workspace_path,
            "Remote",
            Some("trunk()"),
            "trunk()..@ ~ empty()",
            "@..trunk() ~ empty()",
        )
        .await;
    }

    Ok(None)
}

async fn remote_bookmark_detail(
    workspace_path: &Path,
    remote_revset: &str,
    bookmark: &str,
) -> Result<Option<String>, JjError> {
    let output = with_stale_retry(
        workspace_path,
        &[
            "log",
            "-r",
            remote_revset,
            "-n",
            "1",
            "--no-graph",
            "-T",
            "remote_bookmarks.map(|b| b.name() ++ \"\\t\" ++ b.remote()).join(\"\\n\") ++ \"\\n\"",
        ],
    )
    .await?;
    let matches = output
        .lines()
        .filter_map(|line| {
            let (name, remote) = line.split_once('\t')?;
            (name == bookmark).then(|| format!("{name}@{remote}"))
        })
        .collect::<Vec<_>>();

    Ok(matches
        .iter()
        .find(|name| !name.ends_with("@git"))
        .cloned()
        .or_else(|| matches.first().cloned()))
}

async fn baseline_relation(
    workspace_path: &Path,
    target_name: &str,
    detail_name: Option<impl Into<String>>,
    ahead_revset: &str,
    behind_revset: &str,
) -> Result<Option<BaselineRelation>, JjError> {
    Ok(Some(BaselineRelation {
        target_name: target_name.to_owned(),
        detail_name: detail_name.map(Into::into),
        ahead_count: count_revset(workspace_path, ahead_revset).await?,
        behind_count: count_revset(workspace_path, behind_revset).await?,
    }))
}

fn primary_relation(
    workspace_name: &str,
    local_relation: &Option<BaselineRelation>,
    remote_relation: &Option<BaselineRelation>,
) -> BaselineRelationTarget {
    if workspace_name == "default" && remote_relation.is_some() {
        BaselineRelationTarget::Remote
    } else if local_relation.is_some() {
        BaselineRelationTarget::Local
    } else if remote_relation.is_some() {
        BaselineRelationTarget::Remote
    } else {
        BaselineRelationTarget::None
    }
}

fn revset_string_literal(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub async fn create_workspace(
    root_path: impl AsRef<Path>,
    destination_path: impl AsRef<Path>,
    workspace_name: &str,
) -> Result<(), JjError> {
    let root_path = root_path.as_ref();
    let root_path_str = path_str(root_path)?;
    let destination_path_str = path_str(destination_path.as_ref())?;
    run_jj(
        root_path,
        &[
            "workspace",
            "add",
            "-R",
            root_path_str,
            "--name",
            workspace_name,
            destination_path_str,
        ],
    )
    .await?;
    Ok(())
}

pub async fn rename_workspace(
    workspace_path: impl AsRef<Path>,
    workspace_name: &str,
) -> Result<(), JjError> {
    let workspace_path = workspace_path.as_ref();
    run_jj(workspace_path, &["workspace", "rename", workspace_name]).await?;
    Ok(())
}

pub async fn forget_workspace(
    root_path: impl AsRef<Path>,
    workspace_name: &str,
) -> Result<(), JjError> {
    let root_path = root_path.as_ref();
    let root_path_str = path_str(root_path)?;
    run_jj(
        root_path,
        &["workspace", "forget", "-R", root_path_str, workspace_name],
    )
    .await?;
    Ok(())
}

async fn workspace_path(root_path: &str, workspace_name: &str) -> Result<String, JjError> {
    let output = run_jj(
        root_path.as_ref(),
        &[
            "workspace",
            "root",
            "-R",
            root_path,
            "--name",
            workspace_name,
        ],
    )
    .await?;
    Ok(tokio::fs::canonicalize(output.trim())
        .await?
        .to_string_lossy()
        .to_string())
}

async fn current_workspace_path(root_path: &str) -> Result<String, JjError> {
    let output = run_jj(root_path.as_ref(), &["workspace", "root", "-R", root_path]).await?;
    Ok(tokio::fs::canonicalize(output.trim())
        .await?
        .to_string_lossy()
        .to_string())
}

async fn current_workspace_change_id(root_path: &str) -> Result<String, JjError> {
    Ok(run_jj(
        root_path.as_ref(),
        &[
            "log",
            "-R",
            root_path,
            "-r",
            "@",
            "--no-graph",
            "-T",
            "change_id.short()",
        ],
    )
    .await?
    .trim()
    .to_owned())
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

pub fn workspace_id_for(root_path: &str, workspace_name: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in root_path.bytes().chain([0]).chain(workspace_name.bytes()) {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("workspace-{hash:016x}")
}

fn path_str(path: &Path) -> Result<&str, JjError> {
    path.to_str()
        .ok_or_else(|| JjError::Command(format!("path is not valid UTF-8: {}", path.display())))
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

    #[test]
    fn workspace_status_revsets_match_desktop_behavior() {
        assert_eq!(
            CONFLICTED_WORKSPACE_REVSET,
            "coalesce(@ ~ empty(), @-) & conflicts()"
        );
        assert_eq!(DEFAULT_WORKSPACE_REVSET, "present(default@)");
        assert_eq!(LOCAL_AHEAD_REVSET, "default@..@ ~ empty()");
        assert_eq!(LOCAL_BEHIND_REVSET, "@..default@ ~ empty()");
    }

    #[test]
    fn summarizes_unified_diff_without_file_headers() {
        let diff = "\
diff --git a/a.txt b/a.txt
--- a/a.txt
+++ b/a.txt
@@ -1,2 +1,3 @@
 unchanged
-old
+new
+another
";

        assert_eq!(
            summarize_unified_diff(diff),
            DiffStats {
                added_lines: 2,
                removed_lines: 1,
            }
        );
    }

    #[test]
    fn parses_invalid_counts_as_zero() {
        assert_eq!(parse_count("12\n"), 12);
        assert_eq!(parse_count("not a number"), 0);
    }

    #[test]
    fn parses_and_classifies_bookmarks() {
        let exact = parse_bookmarks("main, review ,\n");
        let display = parse_bookmarks("base\n");

        assert_eq!(exact, vec!["main".to_owned(), "review".to_owned()]);
        assert_eq!(
            classify_bookmark_relation(&exact, &display),
            WorkspaceBookmarkRelation::Exact
        );
        assert_eq!(
            classify_bookmark_relation(&[], &display),
            WorkspaceBookmarkRelation::Above
        );
        assert_eq!(
            classify_bookmark_relation(&[], &[]),
            WorkspaceBookmarkRelation::None
        );
    }

    #[test]
    fn classifies_workspace_state_from_primary_relation() {
        let same = BaselineRelation {
            target_name: "Default".to_owned(),
            detail_name: Some("default@".to_owned()),
            ahead_count: 0,
            behind_count: 0,
        };
        let ahead = BaselineRelation {
            target_name: "Default".to_owned(),
            detail_name: Some("default@".to_owned()),
            ahead_count: 2,
            behind_count: 0,
        };
        assert_eq!(
            classify_workspace_state(true, BaselineRelationTarget::Local, Some(&ahead)),
            WorkspaceState::Conflicted
        );
        assert_eq!(
            classify_workspace_state(false, BaselineRelationTarget::Remote, Some(&same)),
            WorkspaceState::Published
        );
        assert_eq!(
            classify_workspace_state(false, BaselineRelationTarget::Local, Some(&same)),
            WorkspaceState::MergedLocal
        );
        assert_eq!(
            classify_workspace_state(false, BaselineRelationTarget::Local, Some(&ahead)),
            WorkspaceState::Draft
        );
    }

    #[test]
    fn quotes_revset_string_literals() {
        assert_eq!(revset_string_literal("main"), "\"main\"");
        assert_eq!(revset_string_literal("team/\"x\""), "\"team/\\\"x\\\"\"");
    }
}

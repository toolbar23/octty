use super::*;

const DEFAULT_IGNORED_WORKSPACE_PATH_FRAGMENTS: &[&str] = &[
    "/node_modules/",
    "/.git/",
    "/dist/",
    "/artifacts/",
    "/.cache/",
    "/target/",
    "/build/",
    "/out/",
    "/.idea/",
];

const WORKSPACE_WATCH_IGNORE_ENV_KEYS: &[&str] = &["OCTTY_WORKSPACE_WATCH_IGNORE"];

pub(crate) struct WorkspacePathWatcher {
    pub(crate) path: String,
    pub(crate) _watcher: notify::RecommendedWatcher,
}

pub(crate) fn parse_workspace_watch_ignore_fragments() -> Vec<String> {
    parse_workspace_watch_ignore_fragments_from_values(
        WORKSPACE_WATCH_IGNORE_ENV_KEYS
            .iter()
            .filter_map(|key| std::env::var(key).ok()),
    )
}

pub(crate) fn parse_workspace_watch_ignore_fragments_from_values(
    values: impl IntoIterator<Item = String>,
) -> Vec<String> {
    let mut fragments = DEFAULT_IGNORED_WORKSPACE_PATH_FRAGMENTS
        .iter()
        .map(|fragment| (*fragment).to_owned())
        .collect::<Vec<_>>();

    for value in values {
        for part in value.split([',', '\n']) {
            if let Some(fragment) = normalize_workspace_watch_fragment(part) {
                fragments.push(fragment);
            }
        }
    }

    fragments
}

pub(crate) fn should_ignore_workspace_watch_path(
    path_value: impl AsRef<Path>,
    fragments: &[String],
) -> bool {
    let normalized_path = path_value.as_ref().to_string_lossy().replace('\\', "/");
    let normalized_path_with_trailing_slash = if normalized_path.ends_with('/') {
        normalized_path.clone()
    } else {
        format!("{normalized_path}/")
    };

    fragments.iter().any(|fragment| {
        normalized_path.contains(fragment) || normalized_path_with_trailing_slash.contains(fragment)
    })
}

fn normalize_workspace_watch_fragment(fragment: &str) -> Option<String> {
    let normalized = fragment.trim().replace('\\', "/");
    if normalized.is_empty() {
        None
    } else if normalized.contains('/') {
        Some(normalized)
    } else {
        Some(format!("/{normalized}/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_watch_ignores_common_generated_directories() {
        let fragments = parse_workspace_watch_ignore_fragments_from_values([]);

        assert!(should_ignore_workspace_watch_path(
            "/home/pm/lynx/bear/service/target",
            &fragments
        ));
        assert!(should_ignore_workspace_watch_path(
            "/home/pm/lynx/bear/node_modules/pkg/index.js",
            &fragments
        ));
        assert!(!should_ignore_workspace_watch_path(
            "/home/pm/lynx/bear/.jj/repo/store",
            &fragments
        ));
        assert!(!should_ignore_workspace_watch_path(
            "/home/pm/lynx/bear/src/lib.rs",
            &fragments
        ));
    }

    #[test]
    fn workspace_watch_accepts_custom_ignore_fragments() {
        let fragments = parse_workspace_watch_ignore_fragments_from_values([
            "tmp\ncustom-cache,logs".to_owned(),
        ]);

        assert!(should_ignore_workspace_watch_path(
            "/repo/tmp/file",
            &fragments
        ));
        assert!(should_ignore_workspace_watch_path(
            "/repo/custom-cache/file",
            &fragments
        ));
        assert!(should_ignore_workspace_watch_path(
            "/repo/logs/output",
            &fragments
        ));
        assert!(!should_ignore_workspace_watch_path(
            "/repo/src/main.rs",
            &fragments
        ));
    }
}

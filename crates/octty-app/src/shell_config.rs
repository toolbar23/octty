use super::*;

const DEFAULT_SHELL_TYPES_JSON: &str = include_str!("../../../config/shell-types.json");

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ShellTypeConfigFile {
    #[serde(alias = "shellTypes")]
    pub(crate) shell_types: Vec<ShellTypeConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ShellTypeConfig {
    pub(crate) name: String,
    pub(crate) shortcut: String,
    pub(crate) directory: String,
    pub(crate) command: String,
    #[serde(default, alias = "commandParameters", alias = "command-parameters")]
    pub(crate) command_parameters: Vec<String>,
    #[serde(rename = "on_exit", alias = "on-exit", alias = "on exit")]
    pub(crate) on_exit: TerminalExitBehavior,
    #[serde(alias = "defaultWidthChars", alias = "default-width-chars")]
    pub(crate) default_width_chars: u16,
}

pub(crate) fn load_or_create_shell_type_config() -> anyhow::Result<ShellTypeConfigFile> {
    let path = default_shell_type_config_path();
    if path.exists() {
        return load_shell_type_config(&path);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let config = default_shell_type_config()?;
    fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(config)
}

pub(crate) fn default_shell_type_config() -> anyhow::Result<ShellTypeConfigFile> {
    parse_shell_type_config(DEFAULT_SHELL_TYPES_JSON)
}

pub(crate) fn load_shell_type_config(path: &Path) -> anyhow::Result<ShellTypeConfigFile> {
    parse_shell_type_config(&fs::read_to_string(path)?)
}

pub(crate) fn parse_shell_type_config(input: &str) -> anyhow::Result<ShellTypeConfigFile> {
    let config: ShellTypeConfigFile = serde_json::from_str(input)?;
    if config.shell_types.is_empty() {
        anyhow::bail!("shell type config must define at least one shell type");
    }
    for shell_type in &config.shell_types {
        if shell_type.name.trim().is_empty() {
            anyhow::bail!("shell type config contains an empty name");
        }
        if shell_type.default_width_chars == 0 {
            anyhow::bail!(
                "shell type `{}` must have a positive default_width_chars",
                shell_type.name
            );
        }
    }
    Ok(config)
}

pub(crate) fn default_shell_type_config_path() -> PathBuf {
    env_path("OCTTY_RS_SHELL_TYPES_PATH")
        .or_else(|| env_path("OCTTY_SHELL_TYPES_PATH"))
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
                .join("octty")
                .join("shell-types.json")
        })
}

pub(crate) fn shell_type_cwd(shell_type: &ShellTypeConfig, workspace_path: &str) -> String {
    let directory = shell_type.directory.trim();
    if directory.is_empty() || matches!(directory, "workspace" | ".") {
        return workspace_path.to_owned();
    }

    let path = Path::new(directory);
    if path.is_absolute() {
        path.to_string_lossy().to_string()
    } else {
        Path::new(workspace_path)
            .join(path)
            .to_string_lossy()
            .to_string()
    }
}

pub(crate) fn terminal_kind_for_shell_type(name: &str) -> TerminalKind {
    match name.trim().to_ascii_lowercase().as_str() {
        "codex" => TerminalKind::Codex,
        "jjui" => TerminalKind::Jjui,
        "pi" => TerminalKind::Pi,
        "nvim" => TerminalKind::Nvim,
        _ => TerminalKind::Shell,
    }
}

pub(crate) fn shell_pane_state_for_config(
    shell_type: &ShellTypeConfig,
    workspace_path: &str,
) -> PaneState {
    let kind = terminal_kind_for_shell_type(&shell_type.name);
    let cwd = shell_type_cwd(shell_type, workspace_path);
    let mut pane = create_pane_state(PaneType::Shell, cwd, Some(kind));
    pane.title = shell_type.name.clone();
    if let PanePayload::Terminal(payload) = &mut pane.payload {
        payload.shell_type = shell_type.name.clone();
        payload.command = shell_type.command.clone();
        payload.command_parameters = shell_type.command_parameters.clone();
        payload.on_exit = shell_type.on_exit;
        payload.default_width_chars = shell_type.default_width_chars;
    }
    pane
}

fn env_path(key: &str) -> Option<PathBuf> {
    let value = std::env::var_os(key)?;
    let path = Path::new(&value);
    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
}

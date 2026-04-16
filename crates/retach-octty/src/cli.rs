use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

const DEFAULT_HISTORY: usize = 10000;
const MAX_HISTORY: usize = 1_000_000;

fn parse_history(s: &str) -> Result<usize, String> {
    let val: usize = s.parse().map_err(|e| format!("{e}"))?;
    if val > MAX_HISTORY {
        return Err(format!("history size must be at most {MAX_HISTORY}"));
    }
    Ok(val)
}

#[derive(Clone, Debug, Default, Args, PartialEq, Eq)]
pub struct SpawnArgs {
    /// Working directory used when creating the session
    #[arg(long, value_name = "DIR")]
    pub cwd: Option<PathBuf>,
    /// Command and arguments used when creating the session
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub command: Vec<String>,
}

#[derive(Parser)]
#[command(
    name = "retach-octty",
    version,
    about = "Terminal multiplexer with native scrollback"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Open a session: attach if it exists, create if not
    Open {
        /// Session name
        name: String,
        /// Scrollback history size, 0 to disable (used when creating)
        #[arg(long, default_value_t = DEFAULT_HISTORY, value_parser = parse_history)]
        history: usize,
        #[command(flatten)]
        spawn: SpawnArgs,
    },
    /// Create a new session
    New {
        /// Session name (auto-generated if omitted)
        name: Option<String>,
        /// Scrollback history size (0 to disable)
        #[arg(long, default_value_t = DEFAULT_HISTORY, value_parser = parse_history)]
        history: usize,
        #[command(flatten)]
        spawn: SpawnArgs,
    },
    /// Attach to an existing session
    Attach {
        /// Session name
        name: String,
    },
    /// List active sessions
    List,
    /// Kill a session
    Kill {
        /// Session name
        name: String,
    },
    /// Start the server (internal)
    #[command(hide = true)]
    Server,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_accepts_cwd_and_command_after_separator() {
        let cli = Cli::try_parse_from([
            "retach-octty",
            "open",
            "work",
            "--history",
            "42",
            "--cwd",
            "/tmp/repo",
            "--",
            "jjui",
            "--some-flag",
        ])
        .expect("parse command-aware open");

        let Command::Open {
            name,
            history,
            spawn,
        } = cli.command
        else {
            panic!("expected open command");
        };
        assert_eq!(name, "work");
        assert_eq!(history, 42);
        assert_eq!(
            spawn.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/repo"))
        );
        assert_eq!(spawn.command, vec!["jjui", "--some-flag"]);
    }
}

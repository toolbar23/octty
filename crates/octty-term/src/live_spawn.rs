use super::*;

pub fn spawn_live_terminal(spec: TerminalSessionSpec) -> Result<LiveTerminalHandle, TerminalError> {
    spawn_live_terminal_with_notifier(spec, LiveTerminalSnapshotNotifier::default())
}

pub fn spawn_live_terminal_with_notifier(
    spec: TerminalSessionSpec,
    snapshot_notifier: LiveTerminalSnapshotNotifier,
) -> Result<LiveTerminalHandle, TerminalError> {
    ensure_tmux_config()?;
    let launch = build_tmux_pty_launch(&spec);
    let session_id = launch.session_name.clone();
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: spec.rows,
            cols: spec.cols,
            pixel_width: spec.cols.saturating_mul(DEFAULT_CELL_WIDTH),
            pixel_height: spec.rows.saturating_mul(DEFAULT_CELL_HEIGHT),
        })
        .map_err(|error| TerminalError::Pty(error.to_string()))?;

    let mut command = CommandBuilder::new("tmux");
    command.args(launch.args);
    command.cwd(&spec.cwd);
    command.env("TERM", "xterm-256color");
    command.env_remove("TMUX");
    command.env_remove("TMUX_PANE");

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| TerminalError::Pty(error.to_string()))?;
    drop(pair.slave);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| TerminalError::Pty(error.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| TerminalError::Pty(error.to_string()))?;

    let (command_tx, command_rx) = mpsc::channel();
    let (pty_output_tx, pty_output_rx) = mpsc::channel();
    let (wake_tx, wake_rx) = mpsc::channel();
    let (snapshot_tx, snapshot_rx) = mpsc::channel();

    thread::Builder::new()
        .name(format!("octty-pty-read-{session_id}"))
        .spawn({
            let wake_tx = wake_tx.clone();
            move || read_pty_loop(reader, pty_output_tx, wake_tx)
        })
        .map_err(|error| TerminalError::Pty(error.to_string()))?;

    thread::Builder::new()
        .name(format!("octty-terminal-{session_id}"))
        .spawn({
            let session_id = session_id.clone();
            move || {
                let runtime = LiveTerminalRuntime {
                    spec,
                    session_id,
                    master: pair.master,
                    writer,
                    child,
                    command_rx,
                    pty_output_rx,
                    wake_rx,
                    snapshot_tx,
                    snapshot_notifier,
                };
                if let Err(error) = runtime.run() {
                    eprintln!("[octty-term] live terminal failed: {error}");
                }
            }
        })
        .map_err(|error| TerminalError::Pty(error.to_string()))?;

    Ok(LiveTerminalHandle {
        session_id,
        command_tx,
        wake_tx,
        snapshot_rx,
    })
}

pub fn replay_terminal_bytes(
    session_id: &str,
    bytes: &[u8],
    cols: u16,
    rows: u16,
) -> Result<TerminalGridSnapshot, TerminalError> {
    replay_terminal_steps(session_id, cols, rows, [TerminalReplayStep::Output(bytes)])
}

#[derive(Clone, Copy, Debug)]
pub enum TerminalReplayStep<'a> {
    Output(&'a [u8]),
    Resize { cols: u16, rows: u16 },
}

pub fn replay_terminal_steps<'a>(
    session_id: &str,
    cols: u16,
    rows: u16,
    steps: impl IntoIterator<Item = TerminalReplayStep<'a>>,
) -> Result<TerminalGridSnapshot, TerminalError> {
    let mut terminal = Terminal::new(TerminalOptions {
        cols,
        rows,
        max_scrollback: 10_000,
    })
    .map_err(renderer_error)?;
    terminal
        .resize(
            cols,
            rows,
            u32::from(DEFAULT_CELL_WIDTH),
            u32::from(DEFAULT_CELL_HEIGHT),
        )
        .map_err(renderer_error)?;
    for step in steps {
        match step {
            TerminalReplayStep::Output(bytes) => terminal.vt_write(bytes),
            TerminalReplayStep::Resize { cols, rows } => terminal
                .resize(
                    cols,
                    rows,
                    u32::from(DEFAULT_CELL_WIDTH),
                    u32::from(DEFAULT_CELL_HEIGHT),
                )
                .map_err(renderer_error)?,
        }
    }

    let mut renderer = SnapshotExtractor::new()?;
    renderer.snapshot(session_id, &terminal)
}

fn read_pty_loop(
    mut reader: Box<dyn Read + Send>,
    pty_output_tx: mpsc::Sender<Vec<u8>>,
    wake_tx: mpsc::Sender<LiveTerminalWake>,
) {
    let mut buffer = vec![0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if pty_output_tx.send(buffer[..size].to_vec()).is_err()
                    || wake_tx.send(LiveTerminalWake::PtyOutput).is_err()
                {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

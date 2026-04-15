use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    io::{Read, Write},
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use libghostty_vt::{
    Terminal, TerminalOptions, key,
    render::{CellIterator, Dirty, RenderState, RowIterator},
    style::RgbColor,
    terminal::{
        ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
        PrimaryDeviceAttributes, SecondaryDeviceAttributes, SizeReportSize,
    },
};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::{TerminalError, TerminalSessionSpec, build_tmux_pty_launch, ensure_tmux_config};

const DEFAULT_CELL_WIDTH: u16 = 8;
const DEFAULT_CELL_HEIGHT: u16 = 18;
const MAX_COMMANDS_PER_TICK: usize = 512;
const MAX_INITIAL_SNAPSHOTS: usize = 2;
const LIVE_TERMINAL_IDLE_TIMEOUT: Duration = Duration::from_millis(100);
const LIVE_TERMINAL_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(33);
const LIVE_TERMINAL_INTERACTIVE_OUTPUT_WINDOW: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCellSnapshot {
    pub text: String,
    pub fg: Option<TerminalRgb>,
    pub bg: Option<TerminalRgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalRowSnapshot {
    pub cells: Vec<TerminalCellSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalCursorSnapshot {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalGridSnapshot {
    pub session_id: String,
    pub cols: u16,
    pub rows: u16,
    pub default_fg: TerminalRgb,
    pub default_bg: TerminalRgb,
    pub cursor: Option<TerminalCursorSnapshot>,
    pub rows_data: Vec<TerminalRowSnapshot>,
    pub plain_text: String,
    pub timing: TerminalSnapshotTiming,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalSnapshotTiming {
    pub pty_to_snapshot_micros: Option<u64>,
    pub vt_write_micros: u64,
    pub pty_output_bytes: u64,
    pub snapshot_update_micros: u64,
    pub snapshot_extract_micros: u64,
    pub snapshot_build_micros: u64,
    pub snapshot_cells: u32,
    pub snapshot_text_cells: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTerminalKeyInput {
    pub key: LiveTerminalKey,
    pub text: Option<String>,
    pub modifiers: LiveTerminalModifiers,
    pub unshifted: char,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveTerminalModifiers {
    pub shift: bool,
    pub alt: bool,
    pub control: bool,
    pub platform: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveTerminalKey {
    Character(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Escape,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Space,
    F(u8),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalResize {
    pub cols: u16,
    pub rows: u16,
    pub pixel_width: u16,
    pub pixel_height: u16,
}

pub struct LiveTerminalHandle {
    session_id: String,
    command_tx: mpsc::Sender<LiveTerminalCommand>,
    snapshot_rx: mpsc::Receiver<TerminalGridSnapshot>,
}

#[derive(Clone)]
pub struct LiveTerminalSnapshotNotifier {
    notify: Arc<dyn Fn() + Send + Sync + 'static>,
}

impl LiveTerminalSnapshotNotifier {
    pub fn new(notify: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            notify: Arc::new(notify),
        }
    }

    fn notify(&self) {
        (self.notify)();
    }
}

impl Default for LiveTerminalSnapshotNotifier {
    fn default() -> Self {
        Self::new(|| {})
    }
}

#[derive(Debug)]
enum LiveTerminalCommand {
    Key(LiveTerminalKeyInput),
    Bytes(Vec<u8>),
    Output(Vec<u8>),
    Resize(TerminalResize),
    Scroll(isize),
    Shutdown,
}

impl LiveTerminalHandle {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn send_key(&self, input: LiveTerminalKeyInput) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Key(input))
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn send_bytes(&self, bytes: impl Into<Vec<u8>>) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Bytes(bytes.into()))
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn resize(&self, resize: TerminalResize) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Resize(resize))
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn scroll(&self, lines: isize) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Scroll(lines))
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn drain_snapshots(&mut self) -> Vec<TerminalGridSnapshot> {
        let mut snapshots = Vec::new();
        while let Ok(snapshot) = self.snapshot_rx.try_recv() {
            snapshots.push(snapshot);
        }
        snapshots
    }

    pub fn drain_latest_snapshot(&mut self) -> Option<TerminalGridSnapshot> {
        let mut latest = None;
        while let Ok(snapshot) = self.snapshot_rx.try_recv() {
            latest = Some(snapshot);
        }
        latest
    }
}

impl Drop for LiveTerminalHandle {
    fn drop(&mut self) {
        let _ = self.command_tx.send(LiveTerminalCommand::Shutdown);
    }
}

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
    let (snapshot_tx, snapshot_rx) = mpsc::channel();

    thread::Builder::new()
        .name(format!("octty-pty-read-{session_id}"))
        .spawn({
            let command_tx = command_tx.clone();
            move || read_pty_loop(reader, command_tx)
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
        snapshot_rx,
    })
}

fn read_pty_loop(mut reader: Box<dyn Read + Send>, command_tx: mpsc::Sender<LiveTerminalCommand>) {
    let mut buffer = vec![0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => {
                if command_tx
                    .send(LiveTerminalCommand::Output(buffer[..size].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

struct LiveTerminalRuntime {
    spec: TerminalSessionSpec,
    session_id: String,
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    command_rx: mpsc::Receiver<LiveTerminalCommand>,
    snapshot_tx: mpsc::Sender<TerminalGridSnapshot>,
    snapshot_notifier: LiveTerminalSnapshotNotifier,
}

impl LiveTerminalRuntime {
    fn run(mut self) -> Result<(), TerminalError> {
        let grid_size = Cell::new((self.spec.cols, self.spec.rows));
        let pty_responses = RefCell::new(VecDeque::<Vec<u8>>::new());
        let mut terminal = Terminal::new(TerminalOptions {
            cols: self.spec.cols,
            rows: self.spec.rows,
            max_scrollback: 10_000,
        })
        .map_err(renderer_error)?;

        terminal
            .resize(
                self.spec.cols,
                self.spec.rows,
                u32::from(DEFAULT_CELL_WIDTH),
                u32::from(DEFAULT_CELL_HEIGHT),
            )
            .map_err(renderer_error)?;
        install_terminal_effects(&mut terminal, &grid_size, &pty_responses)?;

        let mut renderer = SnapshotExtractor::new()?;
        let mut input = KeyInputEncoder::new()?;
        let mut terminal_changed = true;
        let mut force_snapshot = true;
        let mut emitted_snapshots = 0usize;
        let mut last_snapshot_at: Option<Instant> = None;
        let mut last_pty_output_at: Option<Instant> = None;
        let mut interactive_output_until: Option<Instant> = None;
        let mut pending_vt_write_micros = 0u64;
        let mut pending_pty_output_bytes = 0u64;

        loop {
            let mut processed_commands = 0usize;
            while processed_commands < MAX_COMMANDS_PER_TICK {
                let command = if processed_commands == 0 {
                    let timeout = terminal_command_wait_timeout(
                        terminal_changed,
                        force_snapshot,
                        emitted_snapshots,
                        last_snapshot_at,
                        Instant::now(),
                    );
                    match self.command_rx.recv_timeout(timeout) {
                        Ok(command) => command,
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                        Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                    }
                } else {
                    match self.command_rx.try_recv() {
                        Ok(command) => command,
                        Err(mpsc::TryRecvError::Empty) => break,
                        Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                    }
                };
                let is_pty_output = matches!(&command, LiveTerminalCommand::Output(_));
                let is_interactive_input = matches!(
                    &command,
                    LiveTerminalCommand::Key(_) | LiveTerminalCommand::Bytes(_)
                );
                let command_received_at = Instant::now();
                processed_commands += 1;
                let effect = self.handle_command(command, &mut terminal, &mut input, &grid_size)?;
                if effect.shutdown {
                    return Ok(());
                }
                pending_vt_write_micros =
                    pending_vt_write_micros.saturating_add(effect.vt_write_micros);
                pending_pty_output_bytes =
                    pending_pty_output_bytes.saturating_add(effect.pty_output_bytes);
                drain_pty_responses(&mut self.writer, &pty_responses)?;
                if is_interactive_input {
                    interactive_output_until =
                        Some(command_received_at + LIVE_TERMINAL_INTERACTIVE_OUTPUT_WINDOW);
                }
                if is_pty_output {
                    last_pty_output_at.get_or_insert(command_received_at);
                    if interactive_output_until
                        .is_some_and(|deadline| command_received_at <= deadline)
                    {
                        force_snapshot = true;
                    }
                }
                terminal_changed |= effect.terminal_changed;
                force_snapshot |= effect.force_snapshot;
            }

            let now = Instant::now();
            if terminal_snapshot_due(
                terminal_changed,
                force_snapshot,
                emitted_snapshots,
                last_snapshot_at,
                now,
            ) {
                let snapshot_started_at = Instant::now();
                let mut snapshot = renderer.snapshot(&self.session_id, &terminal)?;
                let snapshot_finished_at = Instant::now();
                snapshot.timing = TerminalSnapshotTiming {
                    pty_to_snapshot_micros: last_pty_output_at
                        .map(|output_at| micros_since(output_at, snapshot_started_at)),
                    vt_write_micros: pending_vt_write_micros,
                    pty_output_bytes: pending_pty_output_bytes,
                    snapshot_update_micros: snapshot.timing.snapshot_update_micros,
                    snapshot_extract_micros: snapshot.timing.snapshot_extract_micros,
                    snapshot_build_micros: micros_since(snapshot_started_at, snapshot_finished_at),
                    snapshot_cells: snapshot.timing.snapshot_cells,
                    snapshot_text_cells: snapshot.timing.snapshot_text_cells,
                };
                if self.snapshot_tx.send(snapshot).is_ok() {
                    self.snapshot_notifier.notify();
                }
                terminal_changed = false;
                force_snapshot = false;
                last_pty_output_at = None;
                pending_vt_write_micros = 0;
                pending_pty_output_bytes = 0;
                last_snapshot_at = Some(snapshot_started_at);
                emitted_snapshots += 1;
                if emitted_snapshots >= MAX_INITIAL_SNAPSHOTS {
                    renderer.mark_clean(&terminal)?;
                }
            }

            if let Ok(Some(_status)) = self.child.try_wait() {
                return Ok(());
            }
        }
    }

    fn handle_command<'a>(
        &mut self,
        command: LiveTerminalCommand,
        terminal: &mut Terminal<'a, 'a>,
        input: &mut KeyInputEncoder<'a>,
        grid_size: &Cell<(u16, u16)>,
    ) -> Result<LiveTerminalCommandEffect, TerminalError> {
        match command {
            LiveTerminalCommand::Key(key_input) => {
                let bytes = input.encode(terminal, key_input)?;
                if !bytes.is_empty() {
                    self.writer.write_all(&bytes)?;
                    self.writer.flush()?;
                }
                Ok(LiveTerminalCommandEffect::unchanged())
            }
            LiveTerminalCommand::Bytes(bytes) => {
                self.writer.write_all(&bytes)?;
                self.writer.flush()?;
                Ok(LiveTerminalCommandEffect::unchanged())
            }
            LiveTerminalCommand::Output(bytes) => {
                let byte_count = bytes.len() as u64;
                let vt_write_started_at = Instant::now();
                terminal.vt_write(&bytes);
                Ok(LiveTerminalCommandEffect::changed_with_vt_write(
                    micros_since(vt_write_started_at, Instant::now()),
                    byte_count,
                ))
            }
            LiveTerminalCommand::Resize(resize) => {
                let cols = resize.cols.max(1);
                let rows = resize.rows.max(1);
                grid_size.set((cols, rows));
                terminal
                    .resize(
                        cols,
                        rows,
                        u32::from(DEFAULT_CELL_WIDTH),
                        u32::from(DEFAULT_CELL_HEIGHT),
                    )
                    .map_err(renderer_error)?;
                self.master
                    .resize(PtySize {
                        cols,
                        rows,
                        pixel_width: resize.pixel_width,
                        pixel_height: resize.pixel_height,
                    })
                    .map_err(|error| TerminalError::Pty(error.to_string()))?;
                Ok(LiveTerminalCommandEffect::changed_now())
            }
            LiveTerminalCommand::Scroll(lines) => {
                terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Delta(lines));
                Ok(LiveTerminalCommandEffect::changed_now())
            }
            LiveTerminalCommand::Shutdown => Ok(LiveTerminalCommandEffect {
                terminal_changed: false,
                force_snapshot: false,
                shutdown: true,
                vt_write_micros: 0,
                pty_output_bytes: 0,
            }),
        }
    }
}

struct LiveTerminalCommandEffect {
    terminal_changed: bool,
    force_snapshot: bool,
    shutdown: bool,
    vt_write_micros: u64,
    pty_output_bytes: u64,
}

impl LiveTerminalCommandEffect {
    fn changed_with_vt_write(vt_write_micros: u64, pty_output_bytes: u64) -> Self {
        Self {
            terminal_changed: true,
            force_snapshot: false,
            shutdown: false,
            vt_write_micros,
            pty_output_bytes,
        }
    }

    fn changed_now() -> Self {
        Self {
            terminal_changed: true,
            force_snapshot: true,
            shutdown: false,
            vt_write_micros: 0,
            pty_output_bytes: 0,
        }
    }

    fn unchanged() -> Self {
        Self {
            terminal_changed: false,
            force_snapshot: false,
            shutdown: false,
            vt_write_micros: 0,
            pty_output_bytes: 0,
        }
    }
}

fn terminal_command_wait_timeout(
    terminal_changed: bool,
    force_snapshot: bool,
    emitted_snapshots: usize,
    last_snapshot_at: Option<Instant>,
    now: Instant,
) -> Duration {
    if !terminal_changed {
        return LIVE_TERMINAL_IDLE_TIMEOUT;
    }
    if force_snapshot || emitted_snapshots < MAX_INITIAL_SNAPSHOTS {
        return Duration::ZERO;
    }
    last_snapshot_at
        .map(|last_snapshot_at| {
            LIVE_TERMINAL_SNAPSHOT_INTERVAL
                .saturating_sub(now.saturating_duration_since(last_snapshot_at))
        })
        .unwrap_or(Duration::ZERO)
}

fn terminal_snapshot_due(
    terminal_changed: bool,
    force_snapshot: bool,
    emitted_snapshots: usize,
    last_snapshot_at: Option<Instant>,
    now: Instant,
) -> bool {
    terminal_changed
        && terminal_command_wait_timeout(
            terminal_changed,
            force_snapshot,
            emitted_snapshots,
            last_snapshot_at,
            now,
        )
        .is_zero()
}

fn install_terminal_effects<'a>(
    terminal: &mut Terminal<'a, 'a>,
    grid_size: &'a Cell<(u16, u16)>,
    pty_responses: &'a RefCell<VecDeque<Vec<u8>>>,
) -> Result<(), TerminalError> {
    terminal
        .on_pty_write(|_terminal, data| {
            pty_responses.borrow_mut().push_back(data.to_vec());
        })
        .map_err(renderer_error)?
        .on_size(move |_terminal| {
            let (columns, rows) = grid_size.get();
            Some(SizeReportSize {
                rows,
                columns,
                cell_width: u32::from(DEFAULT_CELL_WIDTH),
                cell_height: u32::from(DEFAULT_CELL_HEIGHT),
            })
        })
        .map_err(renderer_error)?
        .on_device_attributes(|_terminal| {
            Some(DeviceAttributes {
                primary: PrimaryDeviceAttributes::new(
                    ConformanceLevel::VT220,
                    [
                        DeviceAttributeFeature::COLUMNS_132,
                        DeviceAttributeFeature::SELECTIVE_ERASE,
                        DeviceAttributeFeature::ANSI_COLOR,
                    ],
                ),
                secondary: SecondaryDeviceAttributes {
                    device_type: DeviceType::VT220,
                    firmware_version: 1,
                    rom_cartridge: 0,
                },
                tertiary: Default::default(),
            })
        })
        .map_err(renderer_error)?
        .on_xtversion(|_terminal| Some("octty-rs"))
        .map_err(renderer_error)?
        .on_color_scheme(|_terminal| None)
        .map_err(renderer_error)?;
    Ok(())
}

fn drain_pty_responses(
    writer: &mut Box<dyn Write + Send>,
    pty_responses: &RefCell<VecDeque<Vec<u8>>>,
) -> Result<(), TerminalError> {
    while let Some(response) = pty_responses.borrow_mut().pop_front() {
        writer.write_all(&response)?;
    }
    writer.flush()?;
    Ok(())
}

struct SnapshotExtractor<'alloc> {
    render_state: RenderState<'alloc>,
    row_iter: RowIterator<'alloc>,
    cell_iter: CellIterator<'alloc>,
}

impl<'alloc> SnapshotExtractor<'alloc> {
    fn new() -> Result<Self, TerminalError> {
        Ok(Self {
            render_state: RenderState::new().map_err(renderer_error)?,
            row_iter: RowIterator::new().map_err(renderer_error)?,
            cell_iter: CellIterator::new().map_err(renderer_error)?,
        })
    }

    fn snapshot(
        &mut self,
        session_id: &str,
        terminal: &Terminal<'alloc, '_>,
    ) -> Result<TerminalGridSnapshot, TerminalError> {
        let update_started_at = Instant::now();
        let snapshot = self.render_state.update(terminal).map_err(renderer_error)?;
        let snapshot_update_micros = micros_since(update_started_at, Instant::now());
        let extract_started_at = Instant::now();
        let colors = snapshot.colors().map_err(renderer_error)?;
        let default_fg = terminal_rgb(colors.foreground);
        let default_bg = terminal_rgb(colors.background);
        let cursor = if snapshot.cursor_visible().map_err(renderer_error)? {
            snapshot
                .cursor_viewport()
                .map_err(renderer_error)?
                .map(|viewport| TerminalCursorSnapshot {
                    col: viewport.x,
                    row: viewport.y,
                    visible: true,
                })
        } else {
            None
        };
        let cols = snapshot.cols().map_err(renderer_error)?;
        let rows = snapshot.rows().map_err(renderer_error)?;
        let mut rows_data = Vec::with_capacity(rows as usize);
        let mut plain_text = String::new();
        let mut row_iteration = self.row_iter.update(&snapshot).map_err(renderer_error)?;
        let mut snapshot_cells = 0u32;
        let mut snapshot_text_cells = 0u32;

        while let Some(row) = row_iteration.next() {
            let mut cells = Vec::with_capacity(cols as usize);
            let mut row_text = String::new();
            let mut cell_iteration = self.cell_iter.update(row).map_err(renderer_error)?;
            while let Some(cell) = cell_iteration.next() {
                let graphemes = cell.graphemes().map_err(renderer_error)?;
                let text: String = graphemes.into_iter().collect();
                let style = cell.style().map_err(renderer_error)?;
                let fg = cell.fg_color().map_err(renderer_error)?.map(terminal_rgb);
                let bg = cell.bg_color().map_err(renderer_error)?.map(terminal_rgb);
                snapshot_cells = snapshot_cells.saturating_add(1);
                if !text.is_empty() {
                    snapshot_text_cells = snapshot_text_cells.saturating_add(1);
                }
                if text.is_empty() {
                    row_text.push(' ');
                } else {
                    row_text.push_str(&text);
                }
                cells.push(TerminalCellSnapshot {
                    text,
                    fg,
                    bg,
                    bold: style.bold,
                    italic: style.italic,
                    underline: !matches!(style.underline, libghostty_vt::style::Underline::None),
                    inverse: style.inverse,
                });
            }
            plain_text.push_str(row_text.trim_end());
            plain_text.push('\n');
            rows_data.push(TerminalRowSnapshot { cells });
        }
        let snapshot_extract_micros = micros_since(extract_started_at, Instant::now());

        Ok(TerminalGridSnapshot {
            session_id: session_id.to_owned(),
            cols,
            rows,
            default_fg,
            default_bg,
            cursor,
            rows_data,
            plain_text,
            timing: TerminalSnapshotTiming {
                snapshot_update_micros,
                snapshot_extract_micros,
                snapshot_cells,
                snapshot_text_cells,
                ..TerminalSnapshotTiming::default()
            },
        })
    }

    fn mark_clean(&mut self, terminal: &Terminal<'alloc, '_>) -> Result<(), TerminalError> {
        let snapshot = self.render_state.update(terminal).map_err(renderer_error)?;
        snapshot.set_dirty(Dirty::Clean).map_err(renderer_error)
    }
}

struct KeyInputEncoder<'alloc> {
    encoder: key::Encoder<'alloc>,
    event: key::Event<'alloc>,
}

impl<'alloc> KeyInputEncoder<'alloc> {
    fn new() -> Result<Self, TerminalError> {
        Ok(Self {
            encoder: key::Encoder::new().map_err(renderer_error)?,
            event: key::Event::new().map_err(renderer_error)?,
        })
    }

    fn encode(
        &mut self,
        terminal: &Terminal<'alloc, '_>,
        input: LiveTerminalKeyInput,
    ) -> Result<Vec<u8>, TerminalError> {
        let mut mods = key::Mods::empty();
        if input.modifiers.shift {
            mods |= key::Mods::SHIFT;
        }
        if input.modifiers.alt {
            mods |= key::Mods::ALT;
        }
        if input.modifiers.control {
            mods |= key::Mods::CTRL;
        }
        if input.modifiers.platform {
            mods |= key::Mods::SUPER;
        }

        let mut consumed_mods = key::Mods::empty();
        if input.text.is_some() && input.modifiers.shift {
            consumed_mods |= key::Mods::SHIFT;
        }

        self.event
            .set_action(key::Action::Press)
            .set_key(key_from_live_key(input.key))
            .set_mods(mods)
            .set_consumed_mods(consumed_mods)
            .set_unshifted_codepoint(input.unshifted)
            .set_utf8(input.text);

        let mut response = Vec::with_capacity(64);
        self.encoder
            .set_options_from_terminal(terminal)
            .encode_to_vec(&self.event, &mut response)
            .map_err(renderer_error)?;
        Ok(response)
    }
}

fn key_from_live_key(key: LiveTerminalKey) -> key::Key {
    match key {
        LiveTerminalKey::Character('a' | 'A') => key::Key::A,
        LiveTerminalKey::Character('b' | 'B') => key::Key::B,
        LiveTerminalKey::Character('c' | 'C') => key::Key::C,
        LiveTerminalKey::Character('d' | 'D') => key::Key::D,
        LiveTerminalKey::Character('e' | 'E') => key::Key::E,
        LiveTerminalKey::Character('f' | 'F') => key::Key::F,
        LiveTerminalKey::Character('g' | 'G') => key::Key::G,
        LiveTerminalKey::Character('h' | 'H') => key::Key::H,
        LiveTerminalKey::Character('i' | 'I') => key::Key::I,
        LiveTerminalKey::Character('j' | 'J') => key::Key::J,
        LiveTerminalKey::Character('k' | 'K') => key::Key::K,
        LiveTerminalKey::Character('l' | 'L') => key::Key::L,
        LiveTerminalKey::Character('m' | 'M') => key::Key::M,
        LiveTerminalKey::Character('n' | 'N') => key::Key::N,
        LiveTerminalKey::Character('o' | 'O') => key::Key::O,
        LiveTerminalKey::Character('p' | 'P') => key::Key::P,
        LiveTerminalKey::Character('q' | 'Q') => key::Key::Q,
        LiveTerminalKey::Character('r' | 'R') => key::Key::R,
        LiveTerminalKey::Character('s' | 'S') => key::Key::S,
        LiveTerminalKey::Character('t' | 'T') => key::Key::T,
        LiveTerminalKey::Character('u' | 'U') => key::Key::U,
        LiveTerminalKey::Character('v' | 'V') => key::Key::V,
        LiveTerminalKey::Character('w' | 'W') => key::Key::W,
        LiveTerminalKey::Character('x' | 'X') => key::Key::X,
        LiveTerminalKey::Character('y' | 'Y') => key::Key::Y,
        LiveTerminalKey::Character('z' | 'Z') => key::Key::Z,
        LiveTerminalKey::Character('0') => key::Key::Digit0,
        LiveTerminalKey::Character('1') => key::Key::Digit1,
        LiveTerminalKey::Character('2') => key::Key::Digit2,
        LiveTerminalKey::Character('3') => key::Key::Digit3,
        LiveTerminalKey::Character('4') => key::Key::Digit4,
        LiveTerminalKey::Character('5') => key::Key::Digit5,
        LiveTerminalKey::Character('6') => key::Key::Digit6,
        LiveTerminalKey::Character('7') => key::Key::Digit7,
        LiveTerminalKey::Character('8') => key::Key::Digit8,
        LiveTerminalKey::Character('9') => key::Key::Digit9,
        LiveTerminalKey::Character('-') => key::Key::Minus,
        LiveTerminalKey::Character('=') => key::Key::Equal,
        LiveTerminalKey::Character('[') => key::Key::BracketLeft,
        LiveTerminalKey::Character(']') => key::Key::BracketRight,
        LiveTerminalKey::Character('\\') => key::Key::Backslash,
        LiveTerminalKey::Character(';') => key::Key::Semicolon,
        LiveTerminalKey::Character('\'') => key::Key::Quote,
        LiveTerminalKey::Character(',') => key::Key::Comma,
        LiveTerminalKey::Character('.') => key::Key::Period,
        LiveTerminalKey::Character('/') => key::Key::Slash,
        LiveTerminalKey::Character('`') => key::Key::Backquote,
        LiveTerminalKey::Character(' ') | LiveTerminalKey::Space => key::Key::Space,
        LiveTerminalKey::Enter => key::Key::Enter,
        LiveTerminalKey::Backspace => key::Key::Backspace,
        LiveTerminalKey::Delete => key::Key::Delete,
        LiveTerminalKey::Tab => key::Key::Tab,
        LiveTerminalKey::Escape => key::Key::Escape,
        LiveTerminalKey::ArrowLeft => key::Key::ArrowLeft,
        LiveTerminalKey::ArrowRight => key::Key::ArrowRight,
        LiveTerminalKey::ArrowUp => key::Key::ArrowUp,
        LiveTerminalKey::ArrowDown => key::Key::ArrowDown,
        LiveTerminalKey::Home => key::Key::Home,
        LiveTerminalKey::End => key::Key::End,
        LiveTerminalKey::PageUp => key::Key::PageUp,
        LiveTerminalKey::PageDown => key::Key::PageDown,
        LiveTerminalKey::Insert => key::Key::Insert,
        LiveTerminalKey::F(1) => key::Key::F1,
        LiveTerminalKey::F(2) => key::Key::F2,
        LiveTerminalKey::F(3) => key::Key::F3,
        LiveTerminalKey::F(4) => key::Key::F4,
        LiveTerminalKey::F(5) => key::Key::F5,
        LiveTerminalKey::F(6) => key::Key::F6,
        LiveTerminalKey::F(7) => key::Key::F7,
        LiveTerminalKey::F(8) => key::Key::F8,
        LiveTerminalKey::F(9) => key::Key::F9,
        LiveTerminalKey::F(10) => key::Key::F10,
        LiveTerminalKey::F(11) => key::Key::F11,
        LiveTerminalKey::F(12) => key::Key::F12,
        LiveTerminalKey::F(_) | LiveTerminalKey::Character(_) => key::Key::Unidentified,
    }
}

fn terminal_rgb(color: RgbColor) -> TerminalRgb {
    TerminalRgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn renderer_error(error: libghostty_vt::Error) -> TerminalError {
    TerminalError::Renderer(error.to_string())
}

fn micros_since(start: Instant, end: Instant) -> u64 {
    end.saturating_duration_since(start)
        .as_micros()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn changed_terminal_waits_until_snapshot_interval() {
        let now = Instant::now();
        let last_snapshot_at = now - Duration::from_millis(10);

        assert_eq!(
            terminal_command_wait_timeout(
                true,
                false,
                MAX_INITIAL_SNAPSHOTS,
                Some(last_snapshot_at),
                now,
            ),
            Duration::from_millis(23)
        );
    }

    #[test]
    fn forced_snapshot_is_due_immediately() {
        let now = Instant::now();

        assert!(terminal_snapshot_due(
            true,
            true,
            MAX_INITIAL_SNAPSHOTS,
            Some(now),
            now,
        ));
    }

    #[test]
    fn idle_terminal_uses_idle_timeout() {
        assert_eq!(
            terminal_command_wait_timeout(
                false,
                false,
                MAX_INITIAL_SNAPSHOTS,
                None,
                Instant::now()
            ),
            LIVE_TERMINAL_IDLE_TIMEOUT
        );
    }

    #[test]
    fn picker_preview_ansi_fixture_reaches_snapshot() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 120,
            rows: 40,
            max_scrollback: 1_000,
        })
        .expect("terminal");
        terminal
            .resize(
                120,
                40,
                u32::from(DEFAULT_CELL_WIDTH),
                u32::from(DEFAULT_CELL_HEIGHT),
            )
            .expect("resize");
        let bytes = picker_preview_ansi_frame(3, 120, 40);
        terminal.vt_write(&bytes);

        let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");
        let snapshot = renderer
            .snapshot("picker-preview-test", &terminal)
            .expect("snapshot");

        assert_eq!(snapshot.cols, 120);
        assert_eq!(snapshot.rows, 40);
        assert!(snapshot.plain_text.contains("preview_"));
        assert!(snapshot.timing.snapshot_cells >= 4_800);
        assert!(snapshot.timing.snapshot_text_cells > 1_000);
    }

    #[test]
    #[ignore = "profiling workload; run with --ignored --nocapture"]
    fn picker_preview_vt_pipeline_profile() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 120,
            rows: 40,
            max_scrollback: 10_000,
        })
        .expect("terminal");
        terminal
            .resize(
                120,
                40,
                u32::from(DEFAULT_CELL_WIDTH),
                u32::from(DEFAULT_CELL_HEIGHT),
            )
            .expect("resize");
        let mut renderer = SnapshotExtractor::new().expect("snapshot extractor");
        let mut vt_write = VecDeque::new();
        let mut update = VecDeque::new();
        let mut extract = VecDeque::new();
        let mut build = VecDeque::new();
        let mut bytes_per_frame = VecDeque::new();
        let mut text_cells = VecDeque::new();

        for frame in 0..180 {
            let bytes = picker_preview_ansi_frame(frame, 120, 40);
            push_test_sample(&mut bytes_per_frame, bytes.len() as u64);
            let vt_started_at = Instant::now();
            terminal.vt_write(&bytes);
            push_test_sample(&mut vt_write, micros_since(vt_started_at, Instant::now()));

            let snapshot_started_at = Instant::now();
            let snapshot = renderer
                .snapshot("picker-preview-profile", &terminal)
                .expect("snapshot");
            push_test_sample(
                &mut build,
                micros_since(snapshot_started_at, Instant::now()),
            );
            push_test_sample(&mut update, snapshot.timing.snapshot_update_micros);
            push_test_sample(&mut extract, snapshot.timing.snapshot_extract_micros);
            push_test_sample(
                &mut text_cells,
                u64::from(snapshot.timing.snapshot_text_cells),
            );
            std::hint::black_box(snapshot);
        }

        println!(
            "picker preview VT pipeline: bytes {} · vt {} · update {} · extract {} · build {} · text cells {}",
            test_count_summary(&bytes_per_frame),
            test_summary(&vt_write),
            test_summary(&update),
            test_summary(&extract),
            test_summary(&build),
            test_count_summary(&text_cells)
        );
    }

    fn picker_preview_ansi_frame(frame: usize, cols: u16, rows: u16) -> Vec<u8> {
        let mut out = String::from("\x1b[?1049h\x1b[?25l\x1b[H");
        let preview_start = 44usize;
        for row in 0..rows as usize {
            let _ = write!(out, "\x1b[{};1H\x1b[0m", row + 1);
            if row == 0 {
                let _ = write!(
                    out,
                    "\x1b[48;2;42;48;56m\x1b[38;2;240;240;240m{:width$}",
                    "  Find files                                      Preview",
                    width = cols as usize
                );
                continue;
            }

            let selected = row == (frame % (rows as usize - 2)) + 1;
            if selected {
                out.push_str("\x1b[48;2;28;92;72m\x1b[38;2;245;250;255m\x1b[1m");
            } else {
                out.push_str("\x1b[0m\x1b[38;2;170;184;194m");
            }
            let _ = write!(
                out,
                "{:<width$}",
                format!(
                    " crates/octty-app/src/{:03}_picker_case.rs ",
                    (frame + row) % 173
                ),
                width = preview_start - 2
            );
            out.push_str("\x1b[0m  ");
            let line_no = (row + frame) % 97;
            let _ = write!(
                out,
                "\x1b[38;2;105;116;126m{line_no:>3} \
                 \x1b[38;2;235;118;135mlet \
                 \x1b[38;2;132;204;244mpreview_{line_no}\
                 \x1b[38;2;210;216;222m = \
                 \x1b[38;2;166;218;149mrender_case({frame}, {row});"
            );
            if row % 5 == 0 {
                let _ = write!(
                    out,
                    "\x1b[{};{}H\x1b[48;2;238;212;132m\x1b[38;2;18;20;22m\x1b[1m changed ",
                    row + 1,
                    preview_start + 5
                );
            }
            out.push_str("\x1b[0m");
        }
        out.into_bytes()
    }

    fn push_test_sample(samples: &mut VecDeque<u64>, micros: u64) {
        if samples.len() == 512 {
            samples.pop_front();
        }
        samples.push_back(micros);
    }

    fn test_summary(samples: &VecDeque<u64>) -> String {
        let mut sorted: Vec<_> = samples.iter().copied().collect();
        sorted.sort_unstable();
        let p50 = sorted[(sorted.len().saturating_sub(1) * 50) / 100];
        let p95 = sorted[(sorted.len().saturating_sub(1) * 95) / 100];
        let max = sorted.last().copied().unwrap_or_default();
        format!(
            "p50 {} p95 {} max {}",
            test_format_micros(p50),
            test_format_micros(p95),
            test_format_micros(max)
        )
    }

    fn test_count_summary(samples: &VecDeque<u64>) -> String {
        let mut sorted: Vec<_> = samples.iter().copied().collect();
        sorted.sort_unstable();
        let p50 = sorted[(sorted.len().saturating_sub(1) * 50) / 100];
        let p95 = sorted[(sorted.len().saturating_sub(1) * 95) / 100];
        let max = sorted.last().copied().unwrap_or_default();
        format!("p50 {p50} p95 {p95} max {max}")
    }

    fn test_format_micros(micros: u64) -> String {
        if micros >= 1_000 {
            format!("{:.1}ms", micros as f64 / 1_000.0)
        } else {
            format!("{micros}us")
        }
    }
}

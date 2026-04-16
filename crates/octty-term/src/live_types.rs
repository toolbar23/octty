#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct TerminalRgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct TerminalCellSnapshot {
    pub text: String,
    pub width: u8,
    pub fg: Option<TerminalRgb>,
    pub bg: Option<TerminalRgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub underline: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
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
    pub scroll: TerminalScrollSnapshot,
    pub default_fg: TerminalRgb,
    pub default_bg: TerminalRgb,
    pub cursor: Option<TerminalCursorSnapshot>,
    pub damage: TerminalDamageSnapshot,
    pub rows_data: Vec<TerminalRowSnapshot>,
    pub plain_text: String,
    pub timing: TerminalSnapshotTiming,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalScrollSnapshot {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalDamageSnapshot {
    pub full: bool,
    pub rows: Vec<u16>,
    pub cells: u32,
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
    pub dirty_rows: u32,
    pub dirty_cells: u32,
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
    pub(crate) session_id: String,
    pub(crate) command_tx: mpsc::Sender<LiveTerminalCommand>,
    pub(crate) wake_tx: mpsc::Sender<LiveTerminalWake>,
    pub(crate) snapshot_rx: mpsc::Receiver<TerminalGridSnapshot>,
    pub(crate) notification_rx: mpsc::Receiver<TerminalNotification>,
    pub(crate) exit_rx: mpsc::Receiver<LiveTerminalExit>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalNotification {
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveTerminalExit {
    pub session_id: String,
    pub exit_code: Option<i64>,
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

    pub(crate) fn notify(&self) {
        (self.notify)();
    }
}

impl Default for LiveTerminalSnapshotNotifier {
    fn default() -> Self {
        Self::new(|| {})
    }
}

#[derive(Debug)]
pub(crate) enum LiveTerminalCommand {
    Key(LiveTerminalKeyInput),
    Bytes(Vec<u8>),
    Resize(TerminalResize),
    Scroll(isize),
    Shutdown,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum LiveTerminalWake {
    Control,
    PtyOutput,
}

impl LiveTerminalHandle {
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn send_key(&self, input: LiveTerminalKeyInput) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Key(input))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        self.wake_tx
            .send(LiveTerminalWake::Control)
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn send_bytes(&self, bytes: impl Into<Vec<u8>>) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Bytes(bytes.into()))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        self.wake_tx
            .send(LiveTerminalWake::Control)
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn resize(&self, resize: TerminalResize) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Resize(resize))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        self.wake_tx
            .send(LiveTerminalWake::Control)
            .map_err(|error| TerminalError::Pty(error.to_string()))
    }

    pub fn scroll(&self, lines: isize) -> Result<(), TerminalError> {
        self.command_tx
            .send(LiveTerminalCommand::Scroll(lines))
            .map_err(|error| TerminalError::Pty(error.to_string()))?;
        self.wake_tx
            .send(LiveTerminalWake::Control)
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

    pub fn drain_notifications(&mut self) -> Vec<TerminalNotification> {
        let mut notifications = Vec::new();
        while let Ok(notification) = self.notification_rx.try_recv() {
            notifications.push(notification);
        }
        notifications
    }

    pub fn drain_exits(&mut self) -> Vec<LiveTerminalExit> {
        let mut exits = Vec::new();
        while let Ok(exit) = self.exit_rx.try_recv() {
            exits.push(exit);
        }
        exits
    }
}

impl Drop for LiveTerminalHandle {
    fn drop(&mut self) {
        let _ = self.command_tx.send(LiveTerminalCommand::Shutdown);
        let _ = self.wake_tx.send(LiveTerminalWake::Control);
    }
}
use super::*;

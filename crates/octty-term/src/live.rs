use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions, create_dir_all},
    hash::{DefaultHasher, Hash, Hasher},
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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

use crate::retach::{build_retach_pty_launch, ensure_retach_config, retach_startup_command};
use crate::{TerminalError, TerminalSessionSpec};

const DEFAULT_CELL_WIDTH: u16 = 8;
const DEFAULT_CELL_HEIGHT: u16 = 18;
const MAX_CONTROL_COMMANDS_PER_TICK: usize = 128;
const MAX_PTY_OUTPUT_CHUNKS_PER_TICK: usize = 256;
const MAX_PTY_OUTPUT_BYTES_PER_TICK: usize = 256 * 1024;
const MAX_INITIAL_SNAPSHOTS: usize = 2;
const LIVE_TERMINAL_IDLE_TIMEOUT: Duration = Duration::from_millis(100);
const LIVE_TERMINAL_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(33);
const LIVE_TERMINAL_INTERACTIVE_OUTPUT_WINDOW: Duration = Duration::from_millis(150);

#[path = "live_input.rs"]
mod input;
#[path = "live_notifications.rs"]
mod notifications;
#[path = "live_runtime.rs"]
mod runtime;
#[path = "live_snapshot.rs"]
mod snapshot;
#[path = "live_spawn.rs"]
mod spawn;
#[path = "live_trace.rs"]
mod trace;
#[path = "live_types.rs"]
mod types;
#[path = "live_utils.rs"]
mod utils;

pub use spawn::{
    TerminalReplayStep, replay_terminal_bytes, replay_terminal_steps, spawn_live_terminal,
    spawn_live_terminal_with_notifier,
};
pub use types::{
    LiveTerminalHandle, LiveTerminalKey, LiveTerminalKeyInput, LiveTerminalModifiers,
    LiveTerminalSnapshotNotifier, TerminalCellSnapshot, TerminalCursorSnapshot,
    TerminalDamageSnapshot, TerminalGridSnapshot, TerminalNotification, TerminalResize,
    TerminalRgb, TerminalRowSnapshot, TerminalScrollSnapshot, TerminalSnapshotTiming,
};

pub(crate) use input::*;
pub(crate) use notifications::*;
pub(crate) use runtime::*;
pub(crate) use snapshot::*;
pub(crate) use trace::*;
pub(crate) use types::*;
pub(crate) use utils::*;

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;

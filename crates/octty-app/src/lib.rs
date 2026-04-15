use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use futures::{StreamExt, channel::mpsc};
use gpui::{
    Action, AnyView, App, Application, Bounds, ClipboardEntry, ClipboardItem, Context, Corner,
    Entity, FocusHandle, Font, FontFallbacks, FontFeatures, Hsla, Image, ImageFormat, IntoElement,
    KeyBinding, KeyDownEvent, Menu, MenuItem, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, PathPromptOptions, Pixels, Point, PromptLevel, Render, Rgba,
    ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, TextRun, Window, WindowBounds,
    WindowOptions, anchored, canvas, deferred, div, fill, font, point, prelude::*, px, rgb, rgba,
    size,
};
use gpui_component::{
    Icon, IconName, Root, Sizable,
    input::{Input, InputState},
    scroll::ScrollableElement,
    tag::Tag,
};
use octty_core::{
    ActivityState, PaneActivity, PanePayload, PaneState, PaneType, ProjectRootRecord,
    SessionSnapshot, SessionState, TerminalPanePayload, WorkspaceBookmarkRelation,
    WorkspaceSnapshot, WorkspaceState, WorkspaceSummary, add_pane, create_default_snapshot,
    create_pane_state, derive_workspace_activity, has_recorded_workspace_path,
    layout::{LAYOUT_VERSION, now_ms},
    remove_pane, screen_fingerprint, workspace_shortcut_targets,
};
use octty_jj::{
    create_workspace as jj_create_workspace, discover_workspaces,
    forget_workspace as jj_forget_workspace, read_workspace_status,
    rename_workspace as jj_rename_workspace, resolve_repo_root,
};
use octty_store::{TursoStore, default_store_path};
use octty_term::{
    TerminalSessionSpec, capture_tmux_pane, capture_tmux_pane_by_session, ensure_tmux_session,
    kill_tmux_session,
    live::{
        LiveTerminalHandle, LiveTerminalKey, LiveTerminalKeyInput, LiveTerminalModifiers,
        LiveTerminalSnapshotNotifier, TerminalGridSnapshot, TerminalReplayStep, TerminalResize,
        TerminalRgb, replay_terminal_bytes, replay_terminal_steps, spawn_live_terminal,
        spawn_live_terminal_with_notifier,
    },
    resize_tmux_session, send_tmux_keys, send_tmux_keys_to_session, send_tmux_text,
    send_tmux_text_to_session, stable_tmux_session_name, tmux_session_activity,
};

mod actions;
mod app_activity;
mod app_core;
mod app_live_terminals;
mod app_panes;
mod app_render;
mod bootstrap;
mod cli;
mod gpui_tokio;
mod input;
mod menu;
mod metrics;
mod sidebar;
mod taskspace;
mod terminal_lifecycle;
mod terminal_render_grid;
mod terminal_render_paint;
mod terminal_render_profile;
mod terminal_render_types;
mod terminal_selection;
mod terminal_state;
mod workspace;

pub use cli::run;

pub(crate) use actions::*;
pub(crate) use bootstrap::*;
pub(crate) use input::*;
pub(crate) use menu::{set_workspace_menu, workspace_key_bindings};
pub(crate) use metrics::*;
pub(crate) use sidebar::*;
pub(crate) use taskspace::*;
pub(crate) use terminal_lifecycle::*;
pub(crate) use terminal_render_grid::*;
pub(crate) use terminal_render_paint::*;
pub(crate) use terminal_render_profile::*;
pub(crate) use terminal_render_types::*;
pub(crate) use terminal_selection::*;
pub(crate) use terminal_state::*;
pub(crate) use workspace::*;

const TERMINAL_CELL_WIDTH: f32 = 8.0;
const TERMINAL_CELL_HEIGHT: f32 = 18.0;
const TERMINAL_FONT_SIZE: f32 = 14.0;
const TERMINAL_DEBUG_TIMER_FONT_SIZE: f32 = 10.0;
const TERMINAL_DEBUG_TIMER_LINE_HEIGHT: f32 = 12.0;
const TERMINAL_SURFACE_PADDING_Y: f32 = 16.0;
const TERMINAL_SURFACE_DEBUG_TIMER_MARGIN_BOTTOM: f32 = 4.0;
const TERMINAL_TASKSPACE_VERTICAL_CHROME_HEIGHT: f32 = 176.0;
const WORKSPACE_SIDEBAR_WIDTH: f32 = 280.0;
const TASKSPACE_HORIZONTAL_PADDING: f32 = 48.0;
const TASKSPACE_PANEL_GAP: f32 = 12.0;
const COLUMN_WIDTH_STEP_PX: f64 = 80.0;
const MIN_COLUMN_WIDTH_PX: f64 = 240.0;
const MAX_COLUMN_WIDTH_PX: f64 = 1_600.0;
const DEFAULT_TERMINAL_FONT_FAMILY: &str = "JetBrains Mono";
const TERMINAL_FOCUSED_FRAME_INTERVAL: Duration = Duration::from_millis(8);
const TERMINAL_BACKGROUND_FRAME_INTERVAL: Duration = Duration::from_millis(100);
const TERMINAL_INTERACTIVE_SNAPSHOT_WINDOW: Duration = Duration::from_millis(150);
const TERMINAL_LATENCY_SAMPLE_LIMIT: usize = 256;
const PANE_ACTIVITY_ACTIVE_WINDOW_MS: i64 = 3_000;
const PANE_ACTIVITY_PERSIST_DELAY: Duration = Duration::from_millis(500);
const PANE_ACTIVITY_RECONCILE_INTERVAL: Duration = Duration::from_secs(10);

#[cfg(test)]
mod tests;

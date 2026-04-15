# Octty Architecture

## High-level structure

Octty is a Rust-only workspace. The application is split into focused Cargo crates:

- `octty-app`: GPUI application shell, taskspace UI, input handling, workspace watching, and live terminal orchestration
- `octty-core`: domain types, pane/layout state, activity state, and shortcut assignment
- `octty-store`: local Turso persistence and migrations
- `octty-jj`: JJ workspace discovery and status calculation
- `octty-term`: tmux launch/control, PTY-backed live terminal sessions, and `libghostty-vt` terminal state extraction

Supporting those are:

- Turso/SQLite for persisted app state
- tmux for durable terminal sessions
- JJ and filesystem watchers for workspace status refresh

## Process model

### GPUI application

Entrypoint: [crates/octty-app/src/main.rs](/home/pm/dev/workspac/crates/octty-app/src/main.rs)

Responsibilities:

- start the Tokio runtime used by async app work
- load bootstrap state from the store and JJ
- create the GPUI window and root view
- bind application actions, menus, and keyboard shortcuts
- own the rendered taskspace state and live terminal handles

### App coordination

Core implementation: [crates/octty-app/src/lib.rs](/home/pm/dev/workspac/crates/octty-app/src/lib.rs)

Responsibilities:

- manage project roots and discovered workspaces
- read JJ status
- maintain workspace summaries
- load and save workspace snapshots
- manage note files and note metadata
- create, restore, and close terminal sessions
- reconcile pane activity and terminal attention state
- refresh UI state after filesystem and JJ changes

This is the main application coordinator. There is no separate JavaScript backend or IPC bridge.

### Terminal runtime

Core implementation: [crates/octty-term/src/lib.rs](/home/pm/dev/workspac/crates/octty-term/src/lib.rs)

Responsibilities:

- start and reuse tmux sessions for durable panes
- run live PTY-backed terminal sessions
- feed terminal bytes into `libghostty-vt`
- expose grid snapshots, dirty rows, cursor state, and notifications to `octty-app`
- accept input, resize, capture, and kill commands

Terminal I/O is native Rust code. The old Node sidecar has been removed.

## Terminal architecture

The terminal stack has four distinct responsibilities:

### 1. Process durability: tmux

Octty uses tmux to keep shell-like sessions durable across UI reloads and pane restore.

Important details:

- Octty tmux sessions use a dedicated socket name
- Octty writes its own tmux config
- Octty strips `TMUX` and `TMUX_PANE` from child env

This prevents the app from accidentally inheriting or mutating the user's normal tmux environment.

### 2. PTY transport: Rust live terminal runtime

The Rust terminal runtime launches tmux inside a PTY and forwards:

- output
- input
- resize events
- exit events

### 3. Terminal model: libghostty-vt

Terminal output bytes are parsed by `libghostty-vt`. Octty extracts structured terminal snapshots with rows, cells, dirty-row information, cursor state, and colors.

### 4. Rendering: GPUI

This means:

- tmux owns the durable shell session
- `octty-term` owns I/O transport and terminal state extraction
- `octty-app` owns the GPUI paint model and taskspace layout

Keeping those concerns separate is important. Replaying full shell history into the UI is not a good durability model.

## Persistence model

Store implementation: [crates/octty-store/src/lib.rs](/home/pm/dev/workspac/crates/octty-store/src/lib.rs)

Default DB path:

- `~/.local/share/octty-rs/state.turso`

Main tables:

- `project_roots`
- `workspaces`
- `workspace_snapshots`
- `note_state`
- `session_state`
- `pane_activity`

### What is persisted

`workspaces` stores lightweight sidebar and status data.

`workspace_snapshots` stores the serialized taskspace layout:

- active pane
- panes
- columns
- pinned side columns

`session_state` stores terminal metadata:

- pane-to-session association
- terminal kind
- cwd
- command
- session state
- exit code
- buffered transcript

`note_state` stores note read state and note metadata, while note content itself stays on disk as markdown files.

`pane_activity` stores activity markers used to tell whether terminal panes and workspaces have new unseen output.

## Filesystem model

The filesystem remains the durable home for user-authored workspace content.

### Notes

Notes are stored directly in the workspace as:

- `*.note.md`

The database stores metadata about them, not the authoritative canonical document location.

### Workspaces

Workspace discovery and status come from JJ and the real filesystem, not from app-only metadata.

That keeps Octty aligned with the actual repo state instead of inventing a separate project model.

## Layout model

Layout logic: [crates/octty-core/src/layout.rs](/home/pm/dev/workspac/crates/octty-core/src/layout.rs)

Primary types: [crates/octty-core/src/types.rs](/home/pm/dev/workspac/crates/octty-core/src/types.rs)

The layout model is column-based:

- panes are stored by ID
- columns store ordered pane IDs
- each column has width and stack height fractions
- left and right pinned columns are explicit
- center columns are ordered independently

This lets the app mutate layout without mixing geometry with pane content state.

## UI model

Taskspace rendering: [crates/octty-app/src/taskspace.rs](/home/pm/dev/workspac/crates/octty-app/src/taskspace.rs)

Responsibilities:

- load bootstrap payload and active workspace state
- render sidebar and taskspace
- manage pane focus and keyboard navigation
- connect terminal panes to the terminal runtime
- update shell, note, and diff panes

The UI should be treated as a projection of durable state plus live runtime handles, not as the source of truth for everything.

## Event flow

Typical workspace open flow:

1. app selects a workspace
2. store and JJ data load workspace summary, notes, and saved snapshot
3. app restores terminal payload state from saved session metadata
4. GPUI taskspace mounts panes
5. terminal panes create or reattach to tmux-backed sessions

Typical terminal flow:

1. app creates a terminal pane
2. `octty-term` creates or reuses a tmux session
3. PTY bytes feed `libghostty-vt`
4. dirty terminal snapshots flow back to `octty-app`
5. GPUI repaints the affected terminal rows

## Watching and status refresh

Workspace status is updated from a combination of:

- JJ reads
- filesystem watching
- explicit refresh paths after relevant actions

The primary workspace status is JJ-native rather than Git-style dirty/clean. The service exposes independent markers instead of forcing all work into one state:

- `published`: no non-empty workspace changes are outside remote bookmarks
- `unpublished`: non-empty workspace changes outside remote bookmarks
- `not in default`: non-empty workspace changes not contained in `default@`
- `conflicted`: unresolved conflicts in the effective workspace commit

The current working-copy diff is still tracked separately for the diff pane and for secondary status detail.

Watch paths intentionally ignore heavy/noisy generated directories, but include `.jj` so JJ operations can refresh published/unpublished workspace markers. Ignored paths include:

- `.git`
- `node_modules`
- `dist`
- `target`
- `.cache`

This is a pragmatic compromise to avoid self-inflicted watch storms.

## Shortcut architecture

Shortcuts are GPUI actions bound at application startup. Workspace navigation, pane creation, pane close, and terminal focus behavior live in the Rust action layer.

## Known tradeoffs

### Terminal rendering

The current terminal renderer uses GPUI paint primitives over snapshots extracted from `libghostty-vt`. This is much closer to a terminal model than the old UI, but [docs/terminal-render-plan.md](/home/pm/dev/workspac/docs/terminal-render-plan.md) still tracks work toward a dedicated batched terminal surface.

### State layering

There is unavoidable complexity because Octty mixes:

- filesystem state
- Turso metadata
- live tmux sessions
- GPUI view state

The architecture works best when those layers stay clearly separated.

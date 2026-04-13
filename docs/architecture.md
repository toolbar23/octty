# Octty Architecture

## High-level structure

Octty has three main layers:

1. Bun backend application logic
2. React renderer for the main view
3. PTY sidecar process for terminal I/O

Supporting those are:

- SQLite for persisted app state
- tmux for durable terminal sessions
- JJ and filesystem watchers for workspace status

## Process model

### Electrobun main process

Entrypoint: [src/bun/index.ts](/home/pm/dev/workspac/src/bun/index.ts)

Responsibilities:

- start the local HTTP and WebSocket API
- create the main BrowserWindow
- bootstrap the renderer HTML
- bridge app-level shortcuts through the native shortcut system

This layer is also where headless API mode is exposed for debugging.

### Workspace service

Core implementation: [src/bun/service.ts](/home/pm/dev/workspac/src/bun/service.ts)

Responsibilities:

- manage project roots and discovered workspaces
- read JJ status
- maintain workspace summaries
- load and save workspace snapshots
- manage note files and note metadata
- create, restore, detach, and close terminal sessions
- broadcast updates to renderer clients over WebSocket

This is the main application coordinator.

### PTY sidecar

Entrypoint: [src/pty-host/index.mjs](/home/pm/dev/workspac/src/pty-host/index.mjs)  
Controller: [src/bun/pty-sidecar.ts](/home/pm/dev/workspac/src/bun/pty-sidecar.ts)

Responsibilities:

- spawn terminal processes through `node-pty`
- stream terminal output back to the Bun service
- accept input, resize, and kill commands

The sidecar exists so terminal I/O is separated from the rest of the app runtime.

## Terminal architecture

The terminal stack has three distinct responsibilities:

### 1. Process durability: tmux

Octty uses tmux to keep shell-like sessions durable across UI reloads and pane restore.

Important details:

- Octty tmux sessions use a dedicated socket name
- Octty writes its own tmux config
- Octty strips `TMUX` and `TMUX_PANE` from child env

This prevents the app from accidentally inheriting or mutating the user's normal tmux environment.

### 2. PTY transport: node-pty sidecar

The PTY sidecar launches tmux and forwards:

- output
- input
- resize events
- exit events

### 3. Rendering: ghostty-web

Renderer-side terminal panes use ghostty-web to display terminal state and handle input.

This means:

- tmux owns the durable shell session
- the PTY sidecar owns I/O transport
- ghostty-web owns terminal rendering in the pane

Keeping those concerns separate is important. Replaying full shell history into a frontend renderer is not a good durability model.

## Persistence model

SQLite implementation: [src/bun/db.ts](/home/pm/dev/workspac/src/bun/db.ts)

Default DB path:

- `~/.local/share/octty/state.sqlite`

Legacy fallback:

- `~/.local/share/workspace-orbit/state.sqlite`

Main tables:

- `project_roots`
- `workspaces`
- `workspace_snapshots`
- `note_state`
- `browser_refs`
- `session_state`

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

`browser_refs` stores per-pane browser references.

`note_state` stores note read state and note metadata, while note content itself stays on disk as markdown files.

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

Shared layout logic: [src/shared/layout.ts](/home/pm/dev/workspac/src/shared/layout.ts)

Primary types: [src/shared/types.ts](/home/pm/dev/workspac/src/shared/types.ts)

The layout model is column-based:

- panes are stored by ID
- columns store ordered pane IDs
- each column has width and stack height fractions
- left and right pinned columns are explicit
- center columns are ordered independently

This lets the renderer mutate layout without mixing geometry with pane content state.

## Renderer model

Renderer entrypoint: [src/mainview/index.tsx](/home/pm/dev/workspac/src/mainview/index.tsx)

Responsibilities:

- load bootstrap payload and active workspace state
- render sidebar and taskspace
- manage pane focus and keyboard navigation
- connect terminal panes to the terminal runtime
- update browser, note, and diff panes

The renderer should be treated as a projection of durable state plus live runtime handles, not as the source of truth for everything.

## Event flow

Typical workspace open flow:

1. renderer asks backend to open a workspace
2. backend loads workspace summary, notes, and saved snapshot
3. backend restores terminal payload state from saved session metadata
4. renderer mounts panes
5. terminal panes create or reattach to tmux-backed sessions

Typical terminal flow:

1. renderer requests session creation
2. backend asks PTY sidecar to spawn tmux
3. PTY sidecar streams output back
4. backend broadcasts `terminal-output`
5. renderer writes output into ghostty-web

## Watching and status refresh

Workspace status is updated from a combination of:

- JJ reads
- filesystem watching
- explicit refresh paths after relevant actions

The primary workspace color is JJ-native rather than Git-style dirty/clean. The
service classifies each workspace by an effective commit, using the current
working-copy commit when it has content and otherwise the parent commit:

- `published`: reachable from any remote bookmark
- `merged-local`: already contained in another local workspace
- `draft`: still unique to this workspace
- `conflicted`: unresolved conflicts override the other states

The current working-copy diff is still tracked separately for the diff pane and
for secondary status detail.

Watch paths intentionally ignore heavy/noisy directories such as:

- `.git`
- `.jj`
- `node_modules`
- `dist`
- `.cache`

This is a pragmatic compromise to avoid self-inflicted watch storms.

## Shortcut architecture

Some app shortcuts are routed through Electrobun's native/global shortcut bridge instead of renderer-only DOM handlers.

Reason:

- embedded browser/webview focus can bypass renderer keyboard handlers

So pane/workspace navigation and pane-close handling need a native interception path to stay reliable.

## Known tradeoffs

### Browser integration

The embedded browser is currently the least stable pane type. It is useful, but more fragile than notes or diff panes.

### Legacy compatibility

The current code still contains compatibility fallbacks for the old product name and old tmux session prefixes so existing local setups are not broken during the rename to Octty.

### State layering

There is unavoidable complexity because Octty mixes:

- filesystem state
- SQLite metadata
- live tmux sessions
- renderer-local view state

The architecture works best when those layers stay clearly separated.

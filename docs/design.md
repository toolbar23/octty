# Octty Design

## What Octty is

Octty is a local-first desktop workspace for terminal-heavy software work.

It is built around a few assumptions:

- the filesystem is the source of truth for projects, workspaces, and notes
- the app should reopen quickly and restore where the user left off
- shells, agents, notes, diffs, and browser references belong in one tiled taskspace
- JJ workspaces are a first-class concept, not an afterthought

The product is closer to a persistent workspace runtime than to a single terminal window.

## Primary user model

The user works with:

- project roots
- JJ workspaces under those roots
- panes inside each workspace taskspace

The left sidebar is the workspace switcher and status surface.
The right side is the active taskspace.

Each workspace has its own restorable layout and pane state.

## Core design principles

### 1. Local-first

Octty keeps durable state on the local machine:

- workspace metadata comes from the filesystem and JJ
- note contents live as `*.note.md` files in the workspace directory
- UI snapshots, browser references, note read state, and terminal session metadata live in local SQLite

This keeps the system debuggable and easy to reason about.

### 2. Fast switching

Switching workspaces should feel close to instantaneous.

That means the app should avoid expensive reconstruction on every workspace switch. The intended model is:

- persistent workspace taskspaces
- persistent terminal emulators
- durable tmux-backed shell processes
- cheap reattachment of UI state instead of replaying history

### 3. Tiled, not tabbed

The main interaction model is a tiled taskspace inspired by tiling compositors.

Current pane forms:

- normal columns
- stacked panes within a column
- left and right pinned columns

This matters because Octty is meant for simultaneous context, not single-view replacement.

### 4. Terminal-native

Terminals are the center of the product, not a secondary tool panel.

Current terminal kinds:

- `shell`
- `codex`
- `pi`
- `nvim`
- `jjui`

These are all represented as shell panes with different launch commands.

### 5. Explicit persistence

The app should restore:

- pane layout
- shell and agent sessions
- notes
- browser references
- diff state

Persistence is explicit in the data model instead of being reconstructed from transient UI state alone.

## Workspace model

A project root is a repository root.

A workspace is one JJ workspace associated with that root. Multiple workspaces can exist under the same project root. The sidebar groups workspaces by project root and shows lightweight status such as:

- workspace markers (`published`, `unpublished`, `not in default`, `conflicted`)
- whether the current working-copy commit has changes
- bookmark info
- unread notes
- active agent count
- recent activity

## Pane model

Octty currently supports five pane types:

- shell
- agent-shell
- note
- browser
- diff

The shared layout model stores panes independently from columns. Columns point to pane IDs, which allows panes to move without rewriting the rest of the workspace snapshot.

Default width policy today:

- shell / agent / diff: about one third of viewport width
- note: narrower
- browser: wider

This keeps the common working set practical without manual resizing every time.

## Why tmux for shells

Terminal process persistence and terminal rendering are different problems.

Octty uses tmux for shell/session durability because tmux already solves:

- long-lived shell processes
- reattachment
- capture of current screen state
- resilience across UI reloads

This is cleaner than treating the frontend terminal renderer as the durable source of truth.

## Why ghostty-web

Ghostty-web is used as the in-app terminal renderer.

Its job is rendering and terminal input handling inside the pane. It should not be treated as the durable owner of the shell session. That ownership belongs to tmux and the PTY sidecar.

## Browser pane design

The browser pane is useful for:

- docs
- issue trackers
- local dashboards
- references tied to a workspace

The browser is intentionally embedded as a tool pane, not as a full browser replacement. Its value is contextual persistence inside the same taskspace as the shells and notes.

## Current rough edges

Some areas are still MVP-grade:

- browser focus can interfere with app-level shortcut behavior, so some shortcuts are routed through the native shortcut bridge
- the browser implementation is more fragile than the note/diff/shell panes
- terminal restore and workspace switching have been under active stabilization

These are implementation constraints, not product goals.

## Design direction

The long-term shape is:

- stable workspace switching
- persistent and isolated pane runtimes
- tmux-backed terminal durability
- filesystem-native notes
- lightweight local metadata cache
- a taskspace that feels closer to a tiling desktop than to a tabbed IDE

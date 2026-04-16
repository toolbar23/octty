# Octty Design

## What Octty is

Octty is a local-first desktop workspace for terminal-heavy software work.

It is built around a few assumptions:

- the filesystem is the source of truth for projects, workspaces, and notes
- the app should reopen quickly and restore where the user left off
- shells, agents, notes, and diffs belong in one tiled taskspace
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
- UI snapshots, note read state, terminal session metadata, and pane activity live in local Turso storage

This keeps the system debuggable and easy to reason about.

### 2. Fast switching

Switching workspaces should feel close to instantaneous.

That means the app should avoid expensive reconstruction on every workspace switch. The intended model is:

- persistent workspace taskspaces
- persistent terminal emulators
- durable retach-backed shell processes
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

Octty currently supports four active pane types:

- shell
- agent-shell
- note
- diff

The shared layout model stores panes independently from columns. Columns point to pane IDs, which allows panes to move without rewriting the rest of the workspace snapshot.

Default width policy today:

- shell / agent / diff: about one third of viewport width
- note: narrower

This keeps the common working set practical without manual resizing every time.

## Why retach for shells

Terminal process persistence and terminal rendering are different problems.

Octty uses retach for shell/session durability because retach solves:

- long-lived shell processes
- reattachment
- scrollback replay into the attached terminal
- resilience across UI reloads

This keeps persistence separate from Octty's UI renderer without adding tmux's scrollback and copy-mode layer.

## Why libghostty-vt

Octty uses `libghostty-vt` as the in-app terminal model.

Its job is parsing terminal bytes and exposing terminal state to the Rust UI. It should not be treated as the durable owner of the shell session. That ownership belongs to retach.

## Current rough edges

Some areas are still MVP-grade:

- note and diff panes are still simple compared with terminal panes
- terminal restore and workspace switching are still under active stabilization
- the GPUI terminal paint path is improving, but a dedicated batched terminal surface may still be needed

These are implementation constraints, not product goals.

## Design direction

The long-term shape is:

- stable workspace switching
- persistent and isolated pane runtimes
- retach-backed terminal durability
- filesystem-native notes
- lightweight local metadata cache
- a taskspace that feels closer to a tiling desktop than to a tabbed IDE

# Octty

Octty is an Electron desktop app for working across many JJ workspaces.

It keeps repo and workspace state on disk, stores cached UI state in a local SQLite database, and restores taskspaces with panes such as:

- shell
- agent
- note
- browser
- diff

## Prerequisites

- Node.js
- npm
- `jj`
- `tmux`

## Development

```bash
npm install
npm run dev
```

Useful commands:

```bash
npm run build
npm run pack:linux
npm run dist:linux
npm run typecheck
npm test
```

## Rust Rewrite Scaffold

The greenfield Rust app lives under `crates/` and is built as a Cargo workspace.
It starts with empty state rather than importing the Electron SQLite database.

```bash
cargo test --workspace --all-targets
cargo run -p octty-app --bin octty
cargo run -p octty-app --bin octty -- --headless-check
cargo run -p octty-app --bin octty -- --bootstrap-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-check.turso cargo run -p octty-app --bin octty -- --pane-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-shell.turso OCTTY_RS_TMUX_SOCKET=octty-rs-check cargo run -p octty-app --bin octty -- --shell-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-io.turso OCTTY_RS_TMUX_SOCKET=octty-rs-io cargo run -p octty-app --bin octty -- --terminal-io-check
```

Rust scaffold pieces:

- `octty-core`: domain types, layout state, workspace shortcut assignment
- `octty-store`: local Turso database and migrations at `~/.local/share/octty-rs/state.turso`
- `octty-jj`: JJ workspace discovery/status helpers
- `octty-term`: tmux launch/session/input/capture plumbing
- `octty-app`: GPUI + gpui-component application shell

The Ghostty terminal adapter is behind an optional feature because
`libghostty-rs` builds vendored Ghostty sources with Zig:

```bash
cargo check -p octty-term --features ghostty-vt
```

That command requires `zig` on `PATH`.

Per-tool launch arguments can be configured with environment variables:

```bash
OCTTY_TERMINAL_ARGS_CODEX='--profile dev --approval-mode "never ask"' npm run dev
OCTTY_TERMINAL_ARGS_PI='--some-flag value' npm run dev
```

Octty inserts these arguments immediately after the executable, so a resumed Codex pane launches as
`codex <your args> resume <session-id>`.

Terminal font rendering can also be tuned from the environment:

```bash
OCTTY_TERMINAL_FONT_FAMILY='"Iosevka Term", monospace' OCTTY_TERMINAL_FONT_SIZE=15 npm run dev
```

## Storage

- App state DB: `~/.local/share/octty/state.sqlite`
- Legacy app state DB: `~/.local/share/workspace-orbit/state.sqlite`
- Notes: `*.note.md` files inside the workspace directory
- Terminal sessions: backed by `tmux`

## Linux Packaging

Build a distributable AppImage with:

```bash
npm install
npm run dist:linux
```

This writes artifacts to `dist/`.

For a quick packaging check without creating an AppImage, build the unpacked Linux app with:

```bash
npm run pack:linux
```

The packaged app still expects `jj` and `tmux` to be available on `PATH` at runtime.

## Current Shape

- Left sidebar: repos and workspaces
- Right side: tiled taskspace with restorable panes
- Workspace metadata comes from the filesystem and JJ

## Docs

- `docs/design.md`
- `docs/architecture.md`

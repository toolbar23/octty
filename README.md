# Octty

Octty is a Rust desktop app for working across many JJ workspaces.

It keeps repo and workspace state on disk, stores cached UI state in local Turso storage, and restores taskspaces with panes such as:

- shell
- agent
- note
- diff

## Prerequisites

- Rust toolchain
- Zig, because the app enables the `libghostty-vt` terminal adapter by default
- `jj`

## Development

```bash
cargo build -p retach-octty
cargo run -p octty-app --bin octty
```

Useful commands:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace --all-targets
cargo build -p retach-octty
cargo run -p octty-app --bin octty -- --headless-check
cargo run -p octty-app --bin octty -- --bootstrap-check
```

## Runtime Checks

These checks exercise the Rust app without needing a normal interactive run:

```bash
cargo run -p octty-app --bin octty -- --headless-check
cargo run -p octty-app --bin octty -- --bootstrap-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-check.turso cargo run -p octty-app --bin octty -- --pane-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-shell.turso cargo run -p octty-app --bin octty -- --shell-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-io.turso cargo run -p octty-app --bin octty -- --terminal-io-check
OCTTY_RS_STATE_PATH=/tmp/octty-rs-live.turso cargo run -p octty-app --bin octty -- --live-terminal-check
```

## Workspace Layout

- `crates/octty-core`: domain types, layout state, workspace shortcut assignment
- `crates/octty-store`: local Turso database and migrations
- `crates/octty-jj`: JJ workspace discovery/status helpers
- `crates/octty-term`: retach launch/session/input/capture plumbing and `libghostty-vt` integration
- `crates/octty-app`: GPUI + gpui-component application shell
- `crates/retach-octty`: vendored retach binary patched for cwd/argv session creation

The terminal adapter is behind an optional feature in `octty-term`, but the main app enables it:

```bash
cargo check -p octty-term --features ghostty-vt
```

Terminal font rendering can also be tuned from the environment:

```bash
OCTTY_RS_TERMINAL_FONT_FAMILY='"Iosevka Term", monospace' cargo run -p octty-app --bin octty
```

Retach integration can be pointed at a non-default binary or history size:

```bash
OCTTY_RETACH_BIN=/path/to/retach-octty OCTTY_RETACH_HISTORY=20000 cargo run -p octty-app --bin octty
```

## Storage

- App state DB: `~/.local/share/octty-rs/state.turso`
- Notes: `*.note.md` files inside the workspace directory
- Terminal sessions: backed by the local `retach-octty` daemon socket

## Current Shape

- Left sidebar: repos and workspaces
- Right side: tiled taskspace with restorable panes
- Workspace metadata comes from the filesystem and JJ

## Docs

- `docs/design.md`
- `docs/architecture.md`
- `docs/terminal-performance.md`
- `docs/terminal-render-plan.md`

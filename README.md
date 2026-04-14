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
npm run typecheck
npm test
```

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

## Current Shape

- Left sidebar: repos and workspaces
- Right side: tiled taskspace with restorable panes
- Workspace metadata comes from the filesystem and JJ

## Docs

- `docs/design.md`
- `docs/architecture.md`

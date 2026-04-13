# Octty

Octty is an Electrobun desktop app for working across many JJ workspaces.

It keeps repo and workspace state on disk, stores cached UI state in a local SQLite database, and restores taskspaces with panes such as:

- shell
- agent
- note
- browser
- diff

## Prerequisites

- Bun
- Electrobun
- `jj`
- `tmux`

## Development

```bash
bun install
bun run dev
```

Useful commands:

```bash
bun run build
bun run typecheck
bun test
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

import { describe, expect, test } from "vitest";
import {
  addPane,
  createDefaultSnapshot,
  defaultColumnWidthForPane,
  findPaneColumnId,
  moveColumn,
  movePaneHorizontally,
  movePaneToColumn,
  movePaneToNewColumn,
  pinColumn,
  removePane,
  resizePaneColumn,
  sanitizeSnapshot,
} from "./layout";

describe("layout helpers", () => {
  test("creates a default shell and diff column layout", () => {
    const snapshot = createDefaultSnapshot("ws-1", "/tmp/demo", 1500);
    const paneTypes = Object.values(snapshot.panes).map((pane) => pane.type).sort();
    const shellPane = Object.values(snapshot.panes).find((pane) => pane.type === "shell")!;
    const diffPane = Object.values(snapshot.panes).find((pane) => pane.type === "diff")!;
    const shellColumn = snapshot.columns[findPaneColumnId(snapshot, shellPane.id)!]!;
    const diffColumn = snapshot.columns[findPaneColumnId(snapshot, diffPane.id)!]!;

    expect(paneTypes).toEqual(["diff", "shell"]);
    expect(snapshot.centerColumnIds).toHaveLength(2);
    expect(snapshot.activePaneId).not.toBeNull();
    expect(snapshot.layoutVersion).toBe(2);
    expect(shellColumn.widthPx).toBe(defaultColumnWidthForPane("shell", 1500));
    expect(diffColumn.widthPx).toBe(defaultColumnWidthForPane("diff", 1500));
  });

  test("adds panes as new columns with pane-type default widths", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo", 1600);
    const withNote = addPane(initial, "note", "/tmp/demo", "new-column", "shell", 1600);
    const notePane = Object.values(withNote.panes).find((pane) => pane.type === "note")!;
    const noteColumn = withNote.columns[findPaneColumnId(withNote, notePane.id)!]!;

    expect(noteColumn.widthPx).toBe(defaultColumnWidthForPane("note", 1600));

    const withDiff = addPane(withNote, "diff", "/tmp/demo", "new-column", "shell", 1600);
    const diffPane = [...Object.values(withDiff.panes)].reverse().find((pane) => pane.type === "diff")!;
    const diffColumn = withDiff.columns[findPaneColumnId(withDiff, diffPane.id)!]!;

    expect(diffColumn.widthPx).toBe(defaultColumnWidthForPane("diff", 1600));
  });

  test("uses viewport-based pane widths", () => {
    expect(defaultColumnWidthForPane("shell", 1500)).toBe(495);
    expect(defaultColumnWidthForPane("diff", 1500)).toBe(495);
    expect(defaultColumnWidthForPane("browser", 1500)).toBe(750);
    expect(defaultColumnWidthForPane("note", 1500)).toBe(225);
  });

  test("stacks a pane into another column and keeps target width", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const next = addPane(initial, "note", "/tmp/demo");
    const shellPane = Object.values(next.panes).find((pane) => pane.type === "shell")!;
    const notePane = Object.values(next.panes).find((pane) => pane.type === "note")!;
    const shellColumnId = findPaneColumnId(next, shellPane.id)!;
    const shellWidth = next.columns[shellColumnId]!.widthPx;

    const stacked = movePaneToColumn(next, notePane.id, shellColumnId);

    expect(stacked.columns[shellColumnId]!.paneIds).toEqual([shellPane.id, notePane.id]);
    expect(stacked.columns[shellColumnId]!.widthPx).toBe(shellWidth);
  });

  test("moves a stacked pane into a new column with the pane default width", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const next = addPane(initial, "note", "/tmp/demo");
    const shellPane = Object.values(next.panes).find((pane) => pane.type === "shell")!;
    const notePane = Object.values(next.panes).find((pane) => pane.type === "note")!;
    const shellColumnId = findPaneColumnId(next, shellPane.id)!;
    const stacked = movePaneToColumn(next, notePane.id, shellColumnId);

    const moved = movePaneToNewColumn(stacked, notePane.id);
    const noteColumnId = findPaneColumnId(moved, notePane.id)!;

    expect(noteColumnId).not.toBe(shellColumnId);
    expect(moved.columns[noteColumnId]!.widthPx).toBe(defaultColumnWidthForPane("note"));
  });

  test("resizes a pane column by delta and clamps the result", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo", 1500);
    const shellPane = Object.values(initial.panes).find((pane) => pane.type === "shell")!;
    const shellColumnId = findPaneColumnId(initial, shellPane.id)!;
    const shellWidth = initial.columns[shellColumnId]!.widthPx;

    const widened = resizePaneColumn(initial, shellPane.id, 80);
    expect(widened.columns[shellColumnId]!.widthPx).toBe(shellWidth + 80);

    const narrowed = resizePaneColumn(initial, shellPane.id, -10_000);
    expect(narrowed.columns[shellColumnId]!.widthPx).toBe(180);
  });

  test("moves a single-pane column left by one slot", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const withNote = addPane(initial, "note", "/tmp/demo");
    const notePane = Object.values(withNote.panes).find((pane) => pane.type === "note")!;
    const noteColumnId = findPaneColumnId(withNote, notePane.id)!;

    const moved = movePaneHorizontally(withNote, notePane.id, -1);

    expect(moved.centerColumnIds).toEqual([
      withNote.centerColumnIds[0]!,
      noteColumnId,
      withNote.centerColumnIds[1]!,
    ]);
  });

  test("splits a stacked pane into a new column when moved horizontally", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo", 1600);
    const withNote = addPane(initial, "note", "/tmp/demo", "new-column", "shell", 1600);
    const shellPane = Object.values(withNote.panes).find((pane) => pane.type === "shell")!;
    const notePane = Object.values(withNote.panes).find((pane) => pane.type === "note")!;
    const shellColumnId = findPaneColumnId(withNote, shellPane.id)!;
    const stacked = movePaneToColumn(withNote, notePane.id, shellColumnId);

    const moved = movePaneHorizontally(stacked, notePane.id, 1, 1600);
    const noteColumnId = findPaneColumnId(moved, notePane.id)!;

    expect(noteColumnId).not.toBe(shellColumnId);
    expect(moved.centerColumnIds).toEqual([shellColumnId, noteColumnId, stacked.centerColumnIds[1]!]);
    expect(moved.columns[noteColumnId]!.widthPx).toBe(defaultColumnWidthForPane("note", 1600));
  });

  test("pins and reorders columns", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const next = addPane(initial, "note", "/tmp/demo");
    const notePane = Object.values(next.panes).find((pane) => pane.type === "note")!;
    const noteColumnId = findPaneColumnId(next, notePane.id)!;

    const pinned = pinColumn(next, noteColumnId, "right");
    expect(pinned.pinnedRightColumnId).toBe(noteColumnId);
    expect(pinned.centerColumnIds).not.toContain(noteColumnId);

    const shellColumnId = findPaneColumnId(
      pinned,
      Object.values(pinned.panes).find((pane) => pane.type === "shell")!.id,
    )!;
    const moved = moveColumn(pinned, shellColumnId, 1);
    expect(moved.centerColumnIds.includes(shellColumnId)).toBe(true);
  });

  test("closing an active pane focuses the next pane when one exists after it", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const shellPane = Object.values(initial.panes).find((pane) => pane.type === "shell")!;
    const orderedPaneIds = initial.centerColumnIds.flatMap(
      (columnId) => initial.columns[columnId]?.paneIds ?? [],
    );

    const closed = removePane(initial, shellPane.id);

    expect(closed.activePaneId).toBe(orderedPaneIds[orderedPaneIds.indexOf(shellPane.id) + 1]!);
  });

  test("closing the last active pane focuses the previous pane", () => {
    const initial = createDefaultSnapshot("ws-1", "/tmp/demo");
    const withNote = addPane(initial, "note", "/tmp/demo");
    const notePane = Object.values(withNote.panes).find((pane) => pane.type === "note")!;
    const orderedPaneIds = withNote.centerColumnIds.flatMap(
      (columnId) => withNote.columns[columnId]?.paneIds ?? [],
    );

    const closed = removePane(withNote, notePane.id);

    expect(closed.activePaneId).toBe(orderedPaneIds[orderedPaneIds.indexOf(notePane.id) - 1]!);
  });

  test("migrates old split snapshots into columns", () => {
    const legacy = {
      workspaceId: "ws-1",
      rootNodeId: "node-root",
      activePaneId: "pane-shell",
      nodes: {
        "node-shell": { id: "node-shell", kind: "pane", paneId: "pane-shell" },
        "node-diff": { id: "node-diff", kind: "pane", paneId: "pane-diff" },
        "node-root": {
          id: "node-root",
          kind: "split",
          direction: "row",
          children: ["node-shell", "node-diff"],
          sizes: [0.5, 0.5],
        },
      },
      panes: {
        "pane-shell": {
          id: "pane-shell",
          type: "shell",
          title: "Shell",
          payload: {
            kind: "shell",
            sessionId: null,
            sessionState: "missing",
            cwd: "/tmp/demo",
            command: "shell",
            exitCode: null,
            autoStart: true,
            restoredBuffer: "",
            embeddedSession: null,
            embeddedSessionCorrelationId: null,
            agentAttentionState: null,
          },
        },
        "pane-diff": {
          id: "pane-diff",
          type: "diff",
          title: "Diff",
          payload: { pinned: false },
        },
      },
      leftSidebarPaneIds: [],
      rightSidebarPaneIds: [],
      collapsedPaneIds: [],
      updatedAt: Date.now(),
    };

    const migrated = sanitizeSnapshot(legacy as never, "/tmp/demo");

    expect(migrated.layoutVersion).toBe(2);
    expect(migrated.centerColumnIds).toHaveLength(2);
    expect(migrated.activePaneId).toBe("pane-shell");
  });
});

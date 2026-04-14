import { describe, expect, test } from "vitest";
import {
  appShortcutActionForKeyEvent,
  workspaceShortcutAccelerator,
  workspaceShortcutTargets,
} from "./app-shortcuts";
import type { ProjectRootRecord, WorkspaceSummary } from "./types";

function makeRoot(overrides: Partial<ProjectRootRecord>): ProjectRootRecord {
  return {
    id: "root-1",
    rootPath: "/tmp/repo",
    displayName: "Repo",
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  };
}

function makeWorkspace(overrides: Partial<WorkspaceSummary>): WorkspaceSummary {
  return {
    id: "workspace-1",
    rootId: "root-1",
    rootPath: "/tmp/repo",
    projectDisplayName: "Repo",
    workspaceName: "default",
    displayName: "default",
    workspacePath: "/tmp/repo",
    workspaceState: "unknown",
    hasWorkingCopyChanges: false,
    effectiveAddedLines: 0,
    effectiveRemovedLines: 0,
    hasConflicts: false,
    unpublishedChangeCount: 0,
    unpublishedAddedLines: 0,
    unpublishedRemovedLines: 0,
    notInDefaultAvailable: false,
    notInDefaultChangeCount: 0,
    notInDefaultAddedLines: 0,
    notInDefaultRemovedLines: 0,
    bookmarks: [],
    bookmarkRelation: "none",
    unreadNotes: 0,
    activeAgentCount: 0,
    agentAttentionState: null,
    recentActivityAt: 0,
    diffText: "",
    createdAt: 1,
    updatedAt: 1,
    lastOpenedAt: 0,
    ...overrides,
  };
}

describe("appShortcutActionForKeyEvent", () => {
  test("maps ctrl-shift pane creation shortcuts", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-shell-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "a",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-codex-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "p",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-pi-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "n",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-nvim-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "j",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-jjui-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "b",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-browser-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "d",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-diff-pane");
  });

  test("keeps existing arrow shortcuts", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowLeft",
        ctrlKey: true,
        shiftKey: false,
        altKey: true,
        metaKey: false,
      }),
    ).toBe("resize-pane-left");
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowRight",
        ctrlKey: true,
        shiftKey: true,
        altKey: true,
        metaKey: false,
      }),
    ).toBe("move-pane-right");
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowUp",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("focus-workspace-up");
  });

  test("maps ctrl-shift number shortcuts to workspace slots", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "1",
        code: "Digit1",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("focus-workspace-1");
    expect(
      appShortcutActionForKeyEvent({
        key: "!",
        code: "Digit1",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("focus-workspace-1");
    expect(
      appShortcutActionForKeyEvent({
        key: "0",
        code: "Digit0",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("focus-workspace-10");
  });

  test("ignores unhandled or disallowed chords", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: false,
        altKey: false,
        metaKey: false,
      }),
    ).toBeNull();
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: true,
      }),
    ).toBeNull();
    expect(
      appShortcutActionForKeyEvent({
        key: "c",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBeNull();
    expect(
      appShortcutActionForKeyEvent({
        key: "x",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBeNull();
  });
});

describe("workspaceShortcutTargets", () => {
  test("returns the first ten available workspaces in sidebar root order", () => {
    const roots = [
      makeRoot({ id: "root-a", displayName: "A" }),
      makeRoot({ id: "root-b", displayName: "B" }),
    ];
    const workspaces = Array.from({ length: 12 }, (_, index) => {
      const rootId = index % 2 === 0 ? "root-b" : "root-a";
      return makeWorkspace({
        id: `workspace-${index + 1}`,
        rootId,
        displayName: `Workspace ${index + 1}`,
        workspacePath: `/tmp/repo-${index + 1}`,
      });
    });
    workspaces[3] = makeWorkspace({
      id: "workspace-missing",
      rootId: "root-a",
      displayName: "Missing",
      workspacePath: "jj-missing://missing",
    });

    const targets = workspaceShortcutTargets(roots, workspaces);

    expect(targets).toHaveLength(10);
    expect(targets.map((target) => target.workspace.rootId).slice(0, 5)).toEqual([
      "root-a",
      "root-a",
      "root-a",
      "root-a",
      "root-a",
    ]);
    expect(targets.map((target) => target.index)).toEqual([1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
    expect(targets.map((target) => target.workspace.id)).not.toContain("workspace-missing");
  });

  test("formats workspace accelerators", () => {
    expect(workspaceShortcutAccelerator(1)).toBe("Ctrl+Shift+1");
    expect(workspaceShortcutAccelerator(10)).toBe("Ctrl+Shift+0");
    expect(workspaceShortcutAccelerator(11)).toBeUndefined();
  });
});

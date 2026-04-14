import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { AppDatabase } from "./db";
import { addPane, createDefaultSnapshot } from "../shared/layout";
import type { WorkspaceSummary } from "../shared/types";

function makeWorkspaceSummary(): WorkspaceSummary {
  return {
    id: "workspace-1",
    rootId: "root-1",
    rootPath: "/tmp/repo",
    projectDisplayName: "repo",
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
  };
}

describe("AppDatabase session state", () => {
  let tempDir: string;
  let db: AppDatabase;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), "octty-db-"));
    db = new AppDatabase(join(tempDir, "state.sqlite"));
    db.upsertProjectRoot({
      id: "root-1",
      rootPath: "/tmp/repo",
      displayName: "repo",
      createdAt: 1,
      updatedAt: 1,
    });
    db.upsertWorkspace(makeWorkspaceSummary());
  });

  afterEach(() => {
    db.close();
    rmSync(tempDir, { recursive: true, force: true });
  });

  test("round-trips persisted terminal buffers by pane", () => {
    db.saveSessionState({
      id: "session-1",
      workspaceId: "workspace-1",
      paneId: "pane-shell",
      kind: "shell",
      cwd: "/tmp/repo",
      command: "/bin/bash",
      buffer: "hello\nworld\n",
      state: "stopped",
      exitCode: 0,
      embeddedSession: null,
      embeddedSessionCorrelationId: null,
      agentAttentionState: null,
    });

    expect(db.getSessionStateByPane("pane-shell")).toEqual({
      id: "session-1",
      workspaceId: "workspace-1",
      paneId: "pane-shell",
      kind: "shell",
      cwd: "/tmp/repo",
      command: "/bin/bash",
      buffer: "hello\nworld\n",
      state: "stopped",
      exitCode: 0,
      embeddedSession: null,
      embeddedSessionCorrelationId: null,
      agentAttentionState: null,
    });
  });

  test("preserves stable terminal session ids when saving a snapshot without a live attachment id", () => {
    const snapshot = addPane(createDefaultSnapshot("workspace-1", "/tmp/repo"), "shell", "/tmp/repo");
    const shellPane = Object.values(snapshot.panes).find((pane) => pane.type === "shell");
    if (!shellPane) {
      throw new Error("Expected shell pane");
    }

    db.saveSessionState({
      id: "session-1",
      workspaceId: "workspace-1",
      paneId: shellPane.id,
      kind: "shell",
      cwd: "/tmp/repo",
      command: "/bin/bash",
      buffer: "persist me",
      state: "live",
      exitCode: null,
      embeddedSession: null,
      embeddedSessionCorrelationId: "octty-embedded-session:1:session-1",
      agentAttentionState: null,
    });

    const terminalPayload = shellPane.payload as {
      sessionId: string | null;
      sessionState: string;
      restoredBuffer: string;
      embeddedSession: unknown;
      embeddedSessionCorrelationId: string | null;
      agentAttentionState: string | null;
    };
    terminalPayload.sessionId = null;
    terminalPayload.sessionState = "missing";
    terminalPayload.restoredBuffer = "";
    terminalPayload.embeddedSession = null;
    terminalPayload.embeddedSessionCorrelationId = null;
    terminalPayload.agentAttentionState = null;

    db.saveSnapshot(snapshot);

    expect(db.getSessionStateByPane(shellPane.id)?.id).toBe("session-1");
  });

  test("does not carry an old transcript into a newly started session snapshot", () => {
    const snapshot = addPane(createDefaultSnapshot("workspace-1", "/tmp/repo"), "shell", "/tmp/repo");
    const shellPane = Object.values(snapshot.panes).find((pane) => pane.type === "shell");
    if (!shellPane) {
      throw new Error("Expected shell pane");
    }

    db.saveSessionState({
      id: "session-old",
      workspaceId: "workspace-1",
      paneId: shellPane.id,
      kind: "shell",
      cwd: "/tmp/repo",
      command: "/bin/bash",
      buffer: "old transcript",
      state: "stopped",
      exitCode: 0,
      embeddedSession: null,
      embeddedSessionCorrelationId: "octty-embedded-session:1:session-old",
      agentAttentionState: null,
    });

    const terminalPayload = shellPane.payload as {
      sessionId: string | null;
      sessionState: string;
      restoredBuffer: string;
      embeddedSession: unknown;
      embeddedSessionCorrelationId: string | null;
      agentAttentionState: string | null;
    };
    terminalPayload.sessionId = "session-new";
    terminalPayload.sessionState = "live";
    terminalPayload.restoredBuffer = "";
    terminalPayload.embeddedSession = null;
    terminalPayload.embeddedSessionCorrelationId = null;
    terminalPayload.agentAttentionState = null;

    db.saveSnapshot(snapshot);

    expect(db.getSessionStateByPane(shellPane.id)?.buffer).toBe("");
    expect(db.getSessionStateByPane(shellPane.id)?.id).toBe("session-new");
  });

  test("round-trips embedded external session metadata", () => {
    db.saveSessionState({
      id: "session-embedded",
      workspaceId: "workspace-1",
      paneId: "pane-shell",
      kind: "codex",
      cwd: "/tmp/repo",
      command: "codex resume embedded-1",
      buffer: "",
      state: "stopped",
      exitCode: 0,
      embeddedSession: {
        provider: "codex",
        id: "embedded-1",
        label: "Saved Codex session",
        detectedAt: 123,
      },
      embeddedSessionCorrelationId: null,
      agentAttentionState: "idle-unseen",
    });

    expect(db.getSessionStateByPane("pane-shell")?.embeddedSession).toEqual({
      provider: "codex",
      id: "embedded-1",
      label: "Saved Codex session",
      detectedAt: 123,
    });
  });

  test("round-trips embedded session correlation ids", () => {
    db.saveSessionState({
      id: "session-correlation",
      workspaceId: "workspace-1",
      paneId: "pane-shell",
      kind: "codex",
      cwd: "/tmp/repo",
      command: "codex",
      buffer: "",
      state: "live",
      exitCode: null,
      embeddedSession: null,
      embeddedSessionCorrelationId: "octty-embedded-session:123:session-correlation",
      agentAttentionState: null,
    });

    expect(db.getSessionStateByPane("pane-shell")?.embeddedSessionCorrelationId).toBe(
      "octty-embedded-session:123:session-correlation",
    );
  });

  test("round-trips agent attention state", () => {
    db.saveSessionState({
      id: "session-attention",
      workspaceId: "workspace-1",
      paneId: "pane-shell",
      kind: "codex",
      cwd: "/tmp/repo",
      command: "codex",
      buffer: "",
      state: "live",
      exitCode: null,
      embeddedSession: null,
      embeddedSessionCorrelationId: null,
      agentAttentionState: "thinking",
    });

    expect(db.getSessionStateByPane("pane-shell")?.agentAttentionState).toBe("thinking");
  });

  test("persists repo and workspace display names separately from JJ names", () => {
    db.updateProjectRootDisplayName("root-1", "Panda");
    db.updateWorkspaceDisplayName("workspace-1", "Review");
    db.updateWorkspaceProjectDisplayName("root-1", "Panda");

    expect(db.listProjectRoots()[0]?.displayName).toBe("Panda");
    expect(db.listWorkspaces()[0]).toMatchObject({
      projectDisplayName: "Panda",
      workspaceName: "default",
      displayName: "Review",
    });
  });

  test("round-trips independent workspace status metrics", () => {
    db.updateWorkspaceStatus("workspace-1", {
      workspaceState: "draft",
      hasConflicts: true,
      unpublishedChangeCount: 3,
      unpublishedAddedLines: 120,
      unpublishedRemovedLines: 8,
      notInDefaultAvailable: true,
      notInDefaultChangeCount: 2,
      notInDefaultAddedLines: 40,
      notInDefaultRemovedLines: 4,
    });

    expect(db.listWorkspaces()[0]).toMatchObject({
      workspaceState: "draft",
      hasConflicts: true,
      unpublishedChangeCount: 3,
      unpublishedAddedLines: 120,
      unpublishedRemovedLines: 8,
      notInDefaultAvailable: true,
      notInDefaultChangeCount: 2,
      notInDefaultAddedLines: 40,
      notInDefaultRemovedLines: 4,
    });
  });
});

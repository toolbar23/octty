import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { AppDatabase } from "./db";
import { createDefaultSnapshot } from "../shared/layout";
import type { WorkspaceSummary } from "../shared/types";

function makeWorkspaceSummary(): WorkspaceSummary {
  return {
    id: "workspace-1",
    rootId: "root-1",
    rootPath: "/tmp/repo",
    projectLabel: "repo",
    workspaceName: "default",
    workspacePath: "/tmp/repo",
    dirty: false,
    bookmarks: [],
    unreadNotes: 0,
    activeAgentCount: 0,
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
      label: "repo",
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
    });
  });

  test("preserves stable terminal session ids when saving a snapshot without a live attachment id", () => {
    const snapshot = createDefaultSnapshot("workspace-1", "/tmp/repo");
    const shellPane = Object.values(snapshot.panes).find((pane) => pane.type === "shell");
    if (!shellPane) {
      throw new Error("Expected default shell pane");
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
    });

    const terminalPayload = shellPane.payload as {
      sessionId: string | null;
      sessionState: string;
      restoredBuffer: string;
      embeddedSession: unknown;
      embeddedSessionCorrelationId: string | null;
    };
    terminalPayload.sessionId = null;
    terminalPayload.sessionState = "missing";
    terminalPayload.restoredBuffer = "";
    terminalPayload.embeddedSession = null;
    terminalPayload.embeddedSessionCorrelationId = null;

    db.saveSnapshot(snapshot);

    expect(db.getSessionStateByPane(shellPane.id)?.id).toBe("session-1");
  });

  test("does not carry an old transcript into a newly started session snapshot", () => {
    const snapshot = createDefaultSnapshot("workspace-1", "/tmp/repo");
    const shellPane = Object.values(snapshot.panes).find((pane) => pane.type === "shell");
    if (!shellPane) {
      throw new Error("Expected default shell pane");
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
    });

    const terminalPayload = shellPane.payload as {
      sessionId: string | null;
      sessionState: string;
      restoredBuffer: string;
      embeddedSession: unknown;
      embeddedSessionCorrelationId: string | null;
    };
    terminalPayload.sessionId = "session-new";
    terminalPayload.sessionState = "live";
    terminalPayload.restoredBuffer = "";
    terminalPayload.embeddedSession = null;
    terminalPayload.embeddedSessionCorrelationId = null;

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
    });

    expect(db.getSessionStateByPane("pane-shell")?.embeddedSessionCorrelationId).toBe(
      "octty-embedded-session:123:session-correlation",
    );
  });
});

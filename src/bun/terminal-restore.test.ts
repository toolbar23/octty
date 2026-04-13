import { describe, expect, test } from "bun:test";
import type { SessionSnapshot, TerminalPanePayload } from "../shared/types";
import { restoreTerminalPanePayload } from "./terminal-restore";

function makePayload(overrides: Partial<TerminalPanePayload> = {}): TerminalPanePayload {
  return {
    kind: "shell",
    sessionId: null,
    sessionState: "missing",
    cwd: "/tmp/repo",
    command: "/bin/bash",
    exitCode: null,
    autoStart: false,
    restoredBuffer: "",
    embeddedSession: null,
    embeddedSessionCorrelationId: null,
    ...overrides,
  };
}

function makeSession(overrides: Partial<SessionSnapshot> = {}): SessionSnapshot {
  return {
    id: "session-1",
    workspaceId: "workspace-1",
    paneId: "pane-1",
    kind: "shell",
    cwd: "/tmp/repo",
    command: "/bin/bash",
    buffer: "prompt$ ",
    state: "live",
    exitCode: null,
    embeddedSession: null,
    embeddedSessionCorrelationId: null,
    ...overrides,
  };
}

describe("restoreTerminalPanePayload", () => {
  test("keeps an actually live attached session live", () => {
    const next = restoreTerminalPanePayload(makePayload(), makeSession(), null);

    expect(next).toMatchObject({
      sessionId: "session-1",
      sessionState: "live",
      autoStart: false,
    });
  });

  test("auto-starts a saved session by stable pane id", () => {
    const next = restoreTerminalPanePayload(makePayload(), null, makeSession());

    expect(next).toMatchObject({
      sessionId: null,
      sessionState: "missing",
      autoStart: true,
    });
  });

  test("restarts a stopped saved session through tmux on restore", () => {
    const next = restoreTerminalPanePayload(
      makePayload(),
      null,
      makeSession({ state: "stopped", exitCode: 0, buffer: "done\n" }),
    );

    expect(next).toMatchObject({
      sessionId: null,
      sessionState: "missing",
      autoStart: true,
      exitCode: 0,
    });
  });

  test("restores an embedded durable session reference from saved state", () => {
    const next = restoreTerminalPanePayload(
      makePayload(),
      null,
      makeSession({
        embeddedSession: {
          provider: "codex",
          id: "session-ext",
          label: "Codex session",
          detectedAt: 1,
        },
      }),
    );

    expect(next.embeddedSession).toEqual({
      provider: "codex",
      id: "session-ext",
      label: "Codex session",
      detectedAt: 1,
    });
  });

  test("restores an embedded session correlation id from saved state", () => {
    const next = restoreTerminalPanePayload(
      makePayload(),
      null,
      makeSession({
        embeddedSessionCorrelationId: "octty-embedded-session:123:session-1",
      }),
    );

    expect(next.embeddedSessionCorrelationId).toBe("octty-embedded-session:123:session-1");
  });

  test("drops stale live handles from older snapshots without auto-restart", () => {
    const next = restoreTerminalPanePayload(
      makePayload({ sessionId: "stale", sessionState: "live" }),
      null,
      null,
    );

    expect(next).toMatchObject({
      sessionId: null,
      sessionState: "stopped",
      autoStart: false,
      exitCode: null,
    });
  });
});

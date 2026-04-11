import type { SessionSnapshot, TerminalPanePayload } from "../shared/types";

export function restoreTerminalPanePayload(
  payload: TerminalPanePayload,
  liveSession: SessionSnapshot | null,
  savedSession: SessionSnapshot | null,
): TerminalPanePayload {
  if (liveSession) {
    return {
      ...payload,
      sessionId: liveSession.id,
      sessionState: liveSession.state,
      autoStart: false,
      exitCode: liveSession.exitCode,
      cwd: liveSession.cwd,
      command: liveSession.command,
    };
  }

  if (savedSession) {
    return {
      ...payload,
      sessionId: null,
      sessionState: "missing",
      autoStart: true,
      exitCode: savedSession.exitCode,
      cwd: savedSession.cwd,
      command: savedSession.command,
    };
  }

  if (payload.sessionId) {
    return {
      ...payload,
      sessionId: null,
      sessionState: "stopped",
      autoStart: false,
      exitCode: payload.exitCode,
    };
  }

  return {
    ...payload,
    sessionState: payload.sessionState || "missing",
  };
}

import type { AgentAttentionState, SessionSnapshot } from "./types";
import { supportsTerminalAttention } from "./terminal-kind";

export function aggregateAgentAttentionStates(
  states: Array<AgentAttentionState | null | undefined>,
): AgentAttentionState | null {
  if (states.some((state) => state === "thinking")) {
    return "thinking";
  }
  if (states.some((state) => state === "idle-unseen")) {
    return "idle-unseen";
  }
  if (states.some((state) => state === "idle-seen")) {
    return "idle-seen";
  }
  return null;
}

export function aggregateWorkspaceAttentionState(
  sessions: Array<Pick<SessionSnapshot, "agentAttentionState" | "kind" | "state">>,
): AgentAttentionState | null {
  return aggregateAgentAttentionStates(
    sessions
      .filter((session) => session.state === "live" && supportsTerminalAttention(session.kind))
      .map((session) => session.agentAttentionState),
  );
}

export function focusedShellAttentionState(
  state: AgentAttentionState | null | undefined,
): AgentAttentionState | null {
  return state === "thinking" ? "thinking" : null;
}

export function settledShellAttentionState(focused: boolean): AgentAttentionState | null {
  return focused ? null : "idle-unseen";
}

export function settledAgentAttentionState(focused: boolean): AgentAttentionState {
  return focused ? "idle-seen" : "idle-unseen";
}

export function agentAttentionLabel(state: AgentAttentionState | null | undefined): string | null {
  switch (state) {
    case "idle-seen":
      return null;
    case "thinking":
      return "working";
    case "idle-unseen":
      return "needs attention";
    default:
      return null;
  }
}

export function agentAttentionClassName(
  state: AgentAttentionState | null | undefined,
): string | null {
  switch (state) {
    case "idle-seen":
      return null;
    case "thinking":
      return "attention-thinking";
    case "idle-unseen":
      return "attention-idle-unseen";
    default:
      return null;
  }
}

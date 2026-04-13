import type { AgentAttentionState } from "./types";

export function aggregateAgentAttentionStates(
  states: Array<AgentAttentionState | null | undefined>,
): AgentAttentionState | null {
  if (states.some((state) => state === "idle-unseen")) {
    return "idle-unseen";
  }
  if (states.some((state) => state === "thinking")) {
    return "thinking";
  }
  if (states.some((state) => state === "idle-seen")) {
    return "idle-seen";
  }
  return null;
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

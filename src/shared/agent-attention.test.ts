import { describe, expect, test } from "vitest";
import {
  agentAttentionClassName,
  agentAttentionLabel,
  aggregateAgentAttentionStates,
  aggregateWorkspaceAttentionState,
} from "./agent-attention";

describe("agent attention helpers", () => {
  test("aggregates active thinking ahead of unseen completion and idle", () => {
    expect(
      aggregateAgentAttentionStates(["idle-seen", "thinking", "idle-unseen"]),
    ).toBe("thinking");
  });

  test("aggregates thinking ahead of seen idle", () => {
    expect(aggregateAgentAttentionStates([null, "idle-seen", "thinking"])).toBe("thinking");
  });

  test("aggregates live shell attention for workspace markers", () => {
    expect(
      aggregateWorkspaceAttentionState([
        { kind: "shell", state: "live", agentAttentionState: "thinking" },
        { kind: "codex", state: "live", agentAttentionState: null },
      ]),
    ).toBe("thinking");
  });

  test("ignores stopped and unsupported sessions for workspace markers", () => {
    expect(
      aggregateWorkspaceAttentionState([
        { kind: "shell", state: "stopped", agentAttentionState: "thinking" },
        { kind: "nvim", state: "live", agentAttentionState: "idle-unseen" },
      ]),
    ).toBeNull();
  });

  test("maps state labels and CSS classes", () => {
    expect(agentAttentionLabel("idle-seen")).toBeNull();
    expect(agentAttentionLabel("thinking")).toBe("working");
    expect(agentAttentionLabel("idle-unseen")).toBe("needs attention");
    expect(agentAttentionClassName("idle-seen")).toBeNull();
    expect(agentAttentionClassName("thinking")).toBe("attention-thinking");
    expect(agentAttentionClassName("idle-unseen")).toBe("attention-idle-unseen");
  });
});

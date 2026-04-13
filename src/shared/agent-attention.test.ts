import { describe, expect, test } from "bun:test";
import {
  agentAttentionClassName,
  agentAttentionLabel,
  aggregateAgentAttentionStates,
} from "./agent-attention";

describe("agent attention helpers", () => {
  test("aggregates unseen completion ahead of thinking and idle", () => {
    expect(
      aggregateAgentAttentionStates(["idle-seen", "thinking", "idle-unseen"]),
    ).toBe("idle-unseen");
  });

  test("aggregates thinking ahead of seen idle", () => {
    expect(aggregateAgentAttentionStates([null, "idle-seen", "thinking"])).toBe("thinking");
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

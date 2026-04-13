import { describe, expect, test } from "bun:test";
import { isAgentTerminalKind, supportsTerminalAttention } from "./terminal-kind";

describe("terminal attention kinds", () => {
  test("keeps agent-only behavior for Codex and Pi", () => {
    expect(isAgentTerminalKind("codex")).toBe(true);
    expect(isAgentTerminalKind("pi")).toBe(true);
    expect(isAgentTerminalKind("shell")).toBe(false);
  });

  test("supports attention markers for shells and agent terminals", () => {
    expect(supportsTerminalAttention("shell")).toBe(true);
    expect(supportsTerminalAttention("codex")).toBe(true);
    expect(supportsTerminalAttention("pi")).toBe(true);
    expect(supportsTerminalAttention("nvim")).toBe(false);
    expect(supportsTerminalAttention("jjui")).toBe(false);
  });
});

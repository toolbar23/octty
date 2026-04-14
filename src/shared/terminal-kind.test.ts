import { describe, expect, test } from "vitest";
import {
  isAgentTerminalKind,
  shouldCloseTerminalPaneOnExit,
  shouldShowTerminalRestart,
  supportsTerminalAttention,
  terminalRestoreRerenderMode,
} from "./terminal-kind";

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

  test("auto-closes terminal panes only after clean exits", () => {
    expect(shouldCloseTerminalPaneOnExit(0)).toBe(true);
    expect(shouldCloseTerminalPaneOnExit(1)).toBe(false);
    expect(shouldCloseTerminalPaneOnExit(null)).toBe(false);
  });

  test("shows restart only after unclean or unknown exits", () => {
    expect(shouldShowTerminalRestart(0)).toBe(false);
    expect(shouldShowTerminalRestart(1)).toBe(true);
    expect(shouldShowTerminalRestart(null)).toBe(true);
  });

  test("defines a restore rerender mode for every terminal kind", () => {
    expect(terminalRestoreRerenderMode("shell")).toBe("resize");
    expect(terminalRestoreRerenderMode("codex")).toBe("resize");
    expect(terminalRestoreRerenderMode("pi")).toBe("resize");
    expect(terminalRestoreRerenderMode("nvim")).toBe("resize");
    expect(terminalRestoreRerenderMode("jjui")).toBe("resize");
  });
});

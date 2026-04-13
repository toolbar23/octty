import { describe, expect, test } from "bun:test";
import {
  isAgentTerminalKind,
  shouldCloseTerminalPaneOnExit,
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

  test("auto-closes only plain shell panes on exit", () => {
    expect(shouldCloseTerminalPaneOnExit("shell")).toBe(true);
    expect(shouldCloseTerminalPaneOnExit("codex")).toBe(false);
    expect(shouldCloseTerminalPaneOnExit("pi")).toBe(false);
    expect(shouldCloseTerminalPaneOnExit("nvim")).toBe(false);
    expect(shouldCloseTerminalPaneOnExit("jjui")).toBe(false);
  });

  test("defines a restore rerender mode for every terminal kind", () => {
    expect(terminalRestoreRerenderMode("shell")).toBe("resize");
    expect(terminalRestoreRerenderMode("codex")).toBe("resize");
    expect(terminalRestoreRerenderMode("pi")).toBe("resize");
    expect(terminalRestoreRerenderMode("nvim")).toBe("resize");
    expect(terminalRestoreRerenderMode("jjui")).toBe("resize");
  });
});

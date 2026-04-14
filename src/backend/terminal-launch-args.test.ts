import { describe, expect, test, vi } from "vitest";
import {
  applyConfiguredTerminalArgs,
  configuredTerminalArgs,
  splitShellWords,
  terminalArgsEnvKey,
} from "./terminal-launch-args";

describe("terminal launch args", () => {
  test("splits shell-style words", () => {
    expect(
      splitShellWords(`--profile dev --prompt "hello world" --label='pane 1' escaped\\ value`),
    ).toEqual(["--profile", "dev", "--prompt", "hello world", "--label=pane 1", "escaped value"]);
  });

  test("loads per-kind args from octty env", () => {
    expect(
      configuredTerminalArgs("codex", {
        [terminalArgsEnvKey("codex")]: `--profile dev --approval-mode "never ask"`,
      }),
    ).toEqual(["--profile", "dev", "--approval-mode", "never ask"]);
  });

  test("ignores invalid shell syntax in configured args", () => {
    const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

    expect(
      configuredTerminalArgs("codex", {
        [terminalArgsEnvKey("codex")]: `--profile "broken`,
      }),
    ).toEqual([]);
    expect(warnSpy).toHaveBeenCalledTimes(1);

    warnSpy.mockRestore();
  });

  test("inserts configured args after the executable", () => {
    expect(
      applyConfiguredTerminalArgs(["codex", "resume", "session-1"], "codex", {
        [terminalArgsEnvKey("codex")]: "--profile dev --sandbox workspace-write",
      }),
    ).toEqual(["codex", "--profile", "dev", "--sandbox", "workspace-write", "resume", "session-1"]);
  });
});

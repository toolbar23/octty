import { describe, expect, test } from "bun:test";
import { shellCommandFor } from "./pty-sidecar";

describe("shellCommandFor", () => {
  test("keeps shell panes as login shells", () => {
    expect(shellCommandFor("shell", "/bin/zsh")).toEqual({
      command: "/bin/zsh",
      args: ["-l"],
    });
  });

  test("wraps codex panes in the user's login shell", () => {
    expect(shellCommandFor("codex", "/bin/zsh", ["codex"])).toEqual({
      command: "/bin/zsh",
      args: ["-lic", "exec codex"],
    });
  });
});

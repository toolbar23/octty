import { describe, expect, test } from "vitest";
import { resolveSidecarWorkingDirectory, shellCommandFor, sidecarPathCandidates } from "./pty-sidecar";

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

  test("includes the packaged runtime sidecar path next to the bundled main process", () => {
    expect(
      sidecarPathCandidates(
        "/repo",
        "/repo",
        "file:///repo/build/electron/main.js",
      ),
    ).toContain("/repo/runtime/pty-host/index.mjs");
  });

  test("falls back to the process cwd when the source root is not a real directory", () => {
    expect(resolveSidecarWorkingDirectory("/repo/resources/app.asar", "/tmp/octty")).toBe(
      "/tmp/octty",
    );
  });
});

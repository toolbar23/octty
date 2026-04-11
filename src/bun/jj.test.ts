import { describe, expect, test } from "bun:test";
import { fallbackWorkspacePath, isStaleWorkingCopyError } from "./jj";

describe("fallbackWorkspacePath", () => {
  test("uses the repo root for the current workspace when JJ has no recorded path", () => {
    expect(
      fallbackWorkspacePath("/home/pm/lynx/panda", "default", "default"),
    ).toBe("/home/pm/lynx/panda");
  });

  test("keeps unrelated missing workspaces marked as missing", () => {
    expect(
      fallbackWorkspacePath("/home/pm/lynx/panda", "panda-frontend", "default"),
    ).toBe("jj-missing://panda-frontend");
  });

  test("detects stale working copy errors", () => {
    expect(
      isStaleWorkingCopyError(
        new Error("The working copy is stale (not updated since operation abc). Hint: Run `jj workspace update-stale` to update it."),
      ),
    ).toBe(true);
    expect(isStaleWorkingCopyError(new Error("some other jj failure"))).toBe(false);
  });
});

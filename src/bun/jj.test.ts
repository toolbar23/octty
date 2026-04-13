import { describe, expect, test } from "bun:test";
import {
  fallbackWorkspacePath,
  isStaleWorkingCopyError,
  withStaleWorkspaceRetry,
} from "./jj";

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

  test("retries once after updating a stale workspace", async () => {
    let attempts = 0;
    const updatedPaths: string[] = [];

    const result = await withStaleWorkspaceRetry(
      "/home/pm/lynx/panda",
      async () => {
        attempts += 1;
        if (attempts === 1) {
          throw new Error(
            "The working copy is stale (not updated since operation abc). Hint: Run `jj workspace update-stale` to update it.",
          );
        }
        return "ok";
      },
      async (workspacePath) => {
        updatedPaths.push(workspacePath);
      },
    );

    expect(result).toBe("ok");
    expect(attempts).toBe(2);
    expect(updatedPaths).toEqual(["/home/pm/lynx/panda"]);
  });

  test("does not update when the failure is unrelated", async () => {
    let updated = false;

    await expect(
      withStaleWorkspaceRetry(
        "/home/pm/lynx/panda",
        async () => {
          throw new Error("some other jj failure");
        },
        async () => {
          updated = true;
        },
      ),
    ).rejects.toThrow("some other jj failure");

    expect(updated).toBe(false);
  });
});

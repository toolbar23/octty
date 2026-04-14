import { describe, expect, test } from "vitest";
import {
  WORKSPACE_STATUS_REVSETS,
  classifyWorkspaceState,
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

  test("uses the repo root when the workspace target matches the current working copy", () => {
    expect(
      fallbackWorkspacePath(
        "/home/pm/lynx/bear",
        "default",
        null,
        "qssvvlqvtosv",
        "qssvvlqvtosv",
      ),
    ).toBe("/home/pm/lynx/bear");
  });

  test("keeps missing workspaces marked as missing when the target does not match", () => {
    expect(
      fallbackWorkspacePath(
        "/home/pm/lynx/bear",
        "default",
        null,
        "qssvvlqvtosv",
        "anotherchange",
      ),
    ).toBe("jj-missing://default");
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

describe("classifyWorkspaceState", () => {
  test("lets conflicts override unpublished state", () => {
    expect(
      classifyWorkspaceState({
        hasConflicts: true,
        unpublishedChangeCount: 3,
      }),
    ).toBe("conflicted");
  });

  test("marks published when there are no unpublished changes", () => {
    expect(
      classifyWorkspaceState({
        hasConflicts: false,
        unpublishedChangeCount: 0,
      }),
    ).toBe("published");
  });

  test("falls back to draft for compatibility when unpublished changes exist", () => {
    expect(
      classifyWorkspaceState({
        hasConflicts: false,
        unpublishedChangeCount: 2,
      }),
    ).toBe("draft");
  });
});

describe("workspace status revsets", () => {
  test("tracks unpublished work independently from current revision state", () => {
    expect(WORKSPACE_STATUS_REVSETS.unpublished).toBe("remote_bookmarks()..@ ~ empty()");
  });

  test("tracks work not contained in the default workspace", () => {
    expect(WORKSPACE_STATUS_REVSETS.defaultWorkspace).toBe("present(default@)");
    expect(WORKSPACE_STATUS_REVSETS.notInDefault).toBe("default@..@ ~ empty()");
  });
});

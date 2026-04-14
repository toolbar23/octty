import { describe, expect, test } from "vitest";
import {
  displayWorkspacePath,
  encodeMissingWorkspacePath,
  hasRecordedWorkspacePath,
} from "./types";

describe("workspace path helpers", () => {
  test("marks sentinel paths as unavailable", () => {
    const missingPath = encodeMissingWorkspacePath("panda-frontend");
    expect(hasRecordedWorkspacePath(missingPath)).toBe(false);
    expect(displayWorkspacePath(missingPath)).toBe("(no recorded path)");
  });

  test("keeps real paths available", () => {
    expect(hasRecordedWorkspacePath("/home/pm/lynx/panda-test")).toBe(true);
    expect(displayWorkspacePath("/home/pm/lynx/panda-test")).toBe("/home/pm/lynx/panda-test");
  });
});

import { describe, expect, test } from "vitest";
import {
  parseWorkspaceWatchIgnoreFragments,
  shouldIgnoreWorkspaceWatchPath,
} from "./workspace-watch";

describe("workspace watch ignores", () => {
  test("ignores common generated directories by default", () => {
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/service/target")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/service/target/classes")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/build")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/build/tmp")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/out")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/out/production")).toBe(true);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/.jj/repo/op_heads")).toBe(false);
    expect(shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/src/main/java")).toBe(false);
  });

  test("parses configurable ignore fragments from env", () => {
    expect(
      parseWorkspaceWatchIgnoreFragments({
        OCTTY_WORKSPACE_WATCH_IGNORE: "coverage, tmp/cache ,/custom/output/",
      }),
    ).toEqual([
      "/node_modules/",
      "/.git/",
      "/dist/",
      "/artifacts/",
      "/.cache/",
      "/target/",
      "/build/",
      "/out/",
      "/.idea/",
      "/coverage/",
      "tmp/cache",
      "/custom/output/",
    ]);
  });

  test("supports the legacy workspace orbit env alias", () => {
    expect(
      shouldIgnoreWorkspaceWatchPath("/home/pm/lynx/bear/reports/generated/index.html", {
        WORKSPACE_ORBIT_WORKSPACE_WATCH_IGNORE: "reports/generated",
      }),
    ).toBe(true);
  });
});

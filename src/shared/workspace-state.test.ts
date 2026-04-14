import { describe, expect, test } from "vitest";
import type { WorkspaceSummary } from "./types";
import { workspaceStatusBadges } from "./workspace-state";

function makeWorkspace(overrides: Partial<WorkspaceSummary> = {}): WorkspaceSummary {
  return {
    id: "workspace-1",
    rootId: "root-1",
    rootPath: "/tmp/repo",
    projectDisplayName: "repo",
    workspaceName: "feature",
    displayName: "feature",
    workspacePath: "/tmp/repo-feature",
    workspaceState: "draft",
    hasWorkingCopyChanges: false,
    effectiveAddedLines: 0,
    effectiveRemovedLines: 0,
    hasConflicts: false,
    unpublishedChangeCount: 0,
    unpublishedAddedLines: 0,
    unpublishedRemovedLines: 0,
    notInDefaultAvailable: false,
    notInDefaultChangeCount: 0,
    notInDefaultAddedLines: 0,
    notInDefaultRemovedLines: 0,
    bookmarks: [],
    bookmarkRelation: "none",
    unreadNotes: 0,
    activeAgentCount: 0,
    agentAttentionState: null,
    recentActivityAt: 0,
    diffText: "",
    createdAt: 0,
    updatedAt: 0,
    lastOpenedAt: 0,
    ...overrides,
  };
}

describe("workspaceStatusBadges", () => {
  test("shows published when there are no unpublished changes", () => {
    expect(
      workspaceStatusBadges(
        makeWorkspace({
          workspaceState: "published",
        }),
      ).map((badge) => badge.label),
    ).toEqual(["Published"]);
  });

  test("does not call unavailable workspace status published", () => {
    expect(
      workspaceStatusBadges(
        makeWorkspace({
          workspaceState: "unknown",
        }),
      ).map((badge) => badge.label),
    ).toEqual(["Unknown"]);
  });

  test("shows unpublished and not-in-default as independent badges", () => {
    expect(
      workspaceStatusBadges(
        makeWorkspace({
          workspaceState: "draft",
          unpublishedChangeCount: 3,
          unpublishedAddedLines: 120,
          unpublishedRemovedLines: 8,
          notInDefaultAvailable: true,
          notInDefaultChangeCount: 2,
          notInDefaultAddedLines: 40,
          notInDefaultRemovedLines: 4,
        }),
    ).map((badge) => badge.label),
    ).toEqual([
      "+120/-8",
      "+40/-4",
    ]);
  });

  test("keeps conflict independent from published and unpublished state", () => {
    expect(
      workspaceStatusBadges(
        makeWorkspace({
          workspaceState: "published",
          hasConflicts: true,
        }),
      ).map((badge) => badge.label),
    ).toEqual([
      "Conflict",
      "Published",
    ]);

    expect(
      workspaceStatusBadges(
        makeWorkspace({
          workspaceState: "draft",
          hasConflicts: true,
          unpublishedChangeCount: 1,
          unpublishedAddedLines: 9,
          unpublishedRemovedLines: 1,
        }),
    ).map((badge) => badge.label),
    ).toEqual([
      "Conflict",
      "+9/-1",
    ]);
  });

  test("does not render draft or latest-revision stats", () => {
    const labels = workspaceStatusBadges(
      makeWorkspace({
        workspaceState: "draft",
        effectiveAddedLines: 221,
        effectiveRemovedLines: 71,
      }),
    ).map((badge) => badge.label);

    expect(labels.join(" ")).not.toContain("Draft");
    expect(labels.join(" ")).not.toContain("+221/-71");
  });
});

import type { WorkspaceState, WorkspaceSummary } from "./types";

export interface WorkspaceStatusBadge {
  label: string;
  className: string;
  title: string;
}

export function workspaceStateClassName(state: WorkspaceState): string {
  return state;
}

export function workspaceStateLabel(state: WorkspaceState): string {
  switch (state) {
    case "published":
      return "Published";
    case "merged-local":
      return "Merged locally";
    case "draft":
      return "Draft";
    case "conflicted":
      return "Conflicted";
    case "unknown":
      return "Unknown";
  }
}

function formatDiffStat(addedLines: number, removedLines: number): string {
  return `+${addedLines}/-${removedLines}`;
}

function formatChangeCount(changeCount: number): string {
  return changeCount === 1 ? "1 change" : `${changeCount} changes`;
}

export function workspaceStatusBadges(workspace: WorkspaceSummary): WorkspaceStatusBadge[] {
  const badges: WorkspaceStatusBadge[] = [];

  if (workspace.workspaceState === "unknown") {
    return [
      {
        label: "Unknown",
        className: "unknown",
        title: "Unknown: workspace status is unavailable.",
      },
    ];
  }

  if (workspace.hasConflicts) {
    badges.push({
      label: "Conflict",
      className: "conflicted",
      title: [
        "Conflict: current workspace state has unresolved conflicts.",
        "revset: coalesce(@ ~ empty(), @-) & conflicts()",
      ].join("\n"),
    });
  }

  if (workspace.unpublishedChangeCount === 0) {
    badges.push({
      label: "Published",
      className: "published",
      title: [
        "Published: all non-empty workspace changes are reachable from remote bookmarks.",
        "revset: remote_bookmarks()..@ ~ empty()",
      ].join("\n"),
    });
  } else {
    badges.push({
      label: formatDiffStat(workspace.unpublishedAddedLines, workspace.unpublishedRemovedLines),
      className: "unpublished",
      title: [
        `Unpublished: ${formatChangeCount(workspace.unpublishedChangeCount)} not reachable from remote bookmarks.`,
        `diff: ${formatDiffStat(workspace.unpublishedAddedLines, workspace.unpublishedRemovedLines)}`,
        "revset: remote_bookmarks()..@ ~ empty()",
      ].join("\n"),
    });
  }

  if (workspace.notInDefaultAvailable && workspace.notInDefaultChangeCount > 0) {
    badges.push({
      label: formatDiffStat(workspace.notInDefaultAddedLines, workspace.notInDefaultRemovedLines),
      className: "not-in-default",
      title: [
        `Not in default: ${formatChangeCount(workspace.notInDefaultChangeCount)} not contained in default@.`,
        `diff: ${formatDiffStat(workspace.notInDefaultAddedLines, workspace.notInDefaultRemovedLines)}`,
        "revset: default@..@ ~ empty()",
      ].join("\n"),
    });
  }

  return badges;
}

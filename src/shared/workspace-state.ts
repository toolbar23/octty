import type { WorkspaceState } from "./types";

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

import {
  hasRecordedWorkspacePath,
  type ProjectRootRecord,
  type WorkspaceSummary,
} from "./types";

export const WORKSPACE_SHORTCUT_LIMIT = 10;

const WORKSPACE_SHORTCUT_DIGITS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"] as const;
type WorkspaceShortcutNumber = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10;
type WorkspaceShortcutAction = `focus-workspace-${WorkspaceShortcutNumber}`;

export type AppShortcutAction =
  | "resize-pane-left"
  | "resize-pane-right"
  | "move-pane-left"
  | "move-pane-right"
  | "focus-pane-left"
  | "focus-pane-right"
  | "focus-workspace-up"
  | "focus-workspace-down"
  | WorkspaceShortcutAction
  | "open-shell-pane"
  | "open-codex-pane"
  | "open-pi-pane"
  | "open-nvim-pane"
  | "open-jjui-pane"
  | "open-browser-pane"
  | "open-diff-pane";

export interface AppShortcutKeyEvent {
  key: string;
  code?: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
}

export interface WorkspaceShortcutTarget {
  workspace: WorkspaceSummary;
  index: WorkspaceShortcutNumber;
}

export function workspaceShortcutDigit(index: number): string | null {
  return WORKSPACE_SHORTCUT_DIGITS[index - 1] ?? null;
}

export function workspaceShortcutLabel(index: number): string | null {
  const digit = workspaceShortcutDigit(index);
  return digit ? `Ctrl+Shift+${digit}` : null;
}

export function workspaceShortcutAccelerator(index: number): string | undefined {
  return workspaceShortcutLabel(index) ?? undefined;
}

export function workspaceShortcutActionForIndex(
  index: number,
): WorkspaceShortcutAction | null {
  if (!Number.isInteger(index) || index < 1 || index > WORKSPACE_SHORTCUT_LIMIT) {
    return null;
  }
  return `focus-workspace-${index as WorkspaceShortcutNumber}`;
}

export function workspaceShortcutIndexForAction(action: string): number | null {
  const match = /^focus-workspace-(\d+)$/.exec(action);
  if (!match) {
    return null;
  }
  const index = Number(match[1]);
  return workspaceShortcutActionForIndex(index) ? index : null;
}

export function workspaceShortcutTargets(
  projectRoots: ProjectRootRecord[],
  workspaces: WorkspaceSummary[],
): WorkspaceShortcutTarget[] {
  const rootIds = new Set(projectRoots.map((root) => root.id));
  const orderedWorkspaces = [
    ...projectRoots.flatMap((root) => workspaces.filter((workspace) => workspace.rootId === root.id)),
    ...workspaces.filter((workspace) => !rootIds.has(workspace.rootId)),
  ];

  return orderedWorkspaces
    .filter((workspace) => hasRecordedWorkspacePath(workspace.workspacePath))
    .slice(0, WORKSPACE_SHORTCUT_LIMIT)
    .map((workspace, offset) => ({
      workspace,
      index: (offset + 1) as WorkspaceShortcutNumber,
    }));
}

function workspaceShortcutIndexForKeyEvent(event: AppShortcutKeyEvent): number | null {
  const codeMatch = /^Digit([0-9])$/.exec(event.code ?? "");
  const digit = codeMatch?.[1] ?? (/^[0-9]$/.test(event.key) ? event.key : null);
  if (!digit) {
    return null;
  }
  const offset = WORKSPACE_SHORTCUT_DIGITS.indexOf(digit as typeof WORKSPACE_SHORTCUT_DIGITS[number]);
  return offset === -1 ? null : offset + 1;
}

export function appShortcutActionForKeyEvent(
  event: AppShortcutKeyEvent,
): AppShortcutAction | null {
  if (!event.ctrlKey || event.metaKey) {
    return null;
  }

  if (
    event.altKey &&
    !event.shiftKey &&
    (event.key === "ArrowLeft" || event.key === "ArrowRight")
  ) {
    return event.key === "ArrowRight" ? "resize-pane-right" : "resize-pane-left";
  }

  if (
    event.shiftKey &&
    event.altKey &&
    (event.key === "ArrowLeft" || event.key === "ArrowRight")
  ) {
    return event.key === "ArrowLeft" ? "move-pane-left" : "move-pane-right";
  }

  if (event.altKey) {
    return null;
  }

  if (event.shiftKey && (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
    return event.key === "ArrowLeft" ? "focus-pane-left" : "focus-pane-right";
  }

  if (event.shiftKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
    return event.key === "ArrowUp" ? "focus-workspace-up" : "focus-workspace-down";
  }

  if (!event.shiftKey) {
    return null;
  }

  const workspaceShortcutIndex = workspaceShortcutIndexForKeyEvent(event);
  if (workspaceShortcutIndex !== null) {
    return workspaceShortcutActionForIndex(workspaceShortcutIndex);
  }

  switch (event.key.toLowerCase()) {
    case "s":
      return "open-shell-pane";
    case "a":
      return "open-codex-pane";
    case "p":
      return "open-pi-pane";
    case "n":
      return "open-nvim-pane";
    case "j":
      return "open-jjui-pane";
    case "b":
      return "open-browser-pane";
    case "d":
      return "open-diff-pane";
    default:
      return null;
  }
}

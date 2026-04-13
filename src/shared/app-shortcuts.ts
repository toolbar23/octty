export type AppShortcutAction =
  | "resize-pane-left"
  | "resize-pane-right"
  | "move-pane-left"
  | "move-pane-right"
  | "focus-pane-left"
  | "focus-pane-right"
  | "focus-workspace-up"
  | "focus-workspace-down"
  | "open-shell-pane"
  | "open-codex-pane"
  | "open-pi-pane"
  | "open-nvim-pane"
  | "open-jjui-pane"
  | "open-diff-pane";

export interface AppShortcutKeyEvent {
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
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

  switch (event.key.toLowerCase()) {
    case "s":
      return "open-shell-pane";
    case "c":
      return "open-codex-pane";
    case "p":
      return "open-pi-pane";
    case "n":
      return "open-nvim-pane";
    case "j":
      return "open-jjui-pane";
    case "d":
      return "open-diff-pane";
    default:
      return null;
  }
}

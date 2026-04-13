export interface TerminalKeyboardShortcutEvent {
  key: string;
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
}

export function shouldRemapShiftEnterToCtrlJ(
  event: TerminalKeyboardShortcutEvent,
): boolean {
  if (event.isComposing || event.ctrlKey || event.altKey || event.metaKey || !event.shiftKey) {
    return false;
  }

  return (
    event.key === "Enter" ||
    event.key === "Return" ||
    event.code === "Enter" ||
    event.code === "NumpadEnter"
  );
}

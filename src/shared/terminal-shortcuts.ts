export interface TerminalKeyboardShortcutEvent {
  key: string;
  code: string;
  shiftKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  isComposing: boolean;
  getModifierState?(keyArg: string): boolean;
}

export type TerminalClipboardShortcutAction = "cut" | "copy" | "paste";

export function terminalClipboardShortcutActionForKeyEvent(
  event: TerminalKeyboardShortcutEvent,
): TerminalClipboardShortcutAction | null {
  if (event.isComposing || event.altKey) {
    return null;
  }

  if (isShiftInsertPaste(event)) {
    return "paste";
  }

  const usesSuperPrefix =
    event.metaKey || event.getModifierState?.("Super") || event.getModifierState?.("OS");
  const usesCtrlShiftPrefix = event.ctrlKey && event.shiftKey && !usesSuperPrefix;
  const usesCommandPrefix = usesSuperPrefix && !event.ctrlKey && !event.shiftKey;
  if (!usesCtrlShiftPrefix && !usesCommandPrefix) {
    return null;
  }

  switch (shortcutLetter(event)) {
    case "x":
      return "cut";
    case "v":
      return "paste";
    case "c":
      return "copy";
    default:
      return null;
  }
}

function isShiftInsertPaste(event: TerminalKeyboardShortcutEvent): boolean {
  return (
    event.shiftKey &&
    !event.ctrlKey &&
    !event.metaKey &&
    !event.getModifierState?.("Super") &&
    !event.getModifierState?.("OS") &&
    (event.key === "Insert" || event.code === "Insert" || event.code === "NumpadInsert")
  );
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

function shortcutLetter(event: Pick<TerminalKeyboardShortcutEvent, "key" | "code">): string {
  const key = event.key.toLowerCase();
  if (key.length === 1) {
    return key;
  }

  if (/^Key[A-Z]$/.test(event.code)) {
    return event.code.slice(3).toLowerCase();
  }

  return key;
}

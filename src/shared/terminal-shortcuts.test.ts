import { describe, expect, test } from "vitest";
import {
  shouldRemapShiftEnterToCtrlJ,
  terminalClipboardShortcutActionForKeyEvent,
} from "./terminal-shortcuts";

describe("shouldRemapShiftEnterToCtrlJ", () => {
  test("matches shifted enter", () => {
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe(true);
  });

  test("matches shifted keypad enter", () => {
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "NumpadEnter",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe(true);
  });

  test("ignores plain enter", () => {
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: false,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe(false);
  });

  test("ignores modified enter chords", () => {
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: true,
        ctrlKey: true,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe(false);
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: true,
        ctrlKey: false,
        altKey: true,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe(false);
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: true,
        isComposing: false,
      }),
    ).toBe(false);
  });

  test("ignores composition events", () => {
    expect(
      shouldRemapShiftEnterToCtrlJ({
        key: "Enter",
        code: "Enter",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: true,
      }),
    ).toBe(false);
  });
});

describe("terminalClipboardShortcutActionForKeyEvent", () => {
  test("uses ctrl-shift as a terminal clipboard prefix", () => {
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "C",
        code: "KeyC",
        shiftKey: true,
        ctrlKey: true,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe("copy");
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "V",
        code: "KeyV",
        shiftKey: true,
        ctrlKey: true,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe("paste");
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "X",
        code: "KeyX",
        shiftKey: true,
        ctrlKey: true,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe("cut");
  });

  test("uses command as a terminal clipboard prefix", () => {
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "c",
        code: "KeyC",
        shiftKey: false,
        ctrlKey: false,
        altKey: false,
        metaKey: true,
        isComposing: false,
      }),
    ).toBe("copy");
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "v",
        code: "KeyV",
        shiftKey: false,
        ctrlKey: false,
        altKey: false,
        metaKey: true,
        isComposing: false,
      }),
    ).toBe("paste");
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "x",
        code: "KeyX",
        shiftKey: false,
        ctrlKey: false,
        altKey: false,
        metaKey: true,
        isComposing: false,
      }),
    ).toBe("cut");
  });

  test("uses super as a terminal clipboard prefix", () => {
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "v",
        code: "KeyV",
        shiftKey: false,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
        getModifierState: (keyArg) => keyArg === "Super",
      }),
    ).toBe("paste");
  });

  test("uses shift-insert as terminal paste", () => {
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "Insert",
        code: "Insert",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe("paste");
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "Insert",
        code: "NumpadInsert",
        shiftKey: true,
        ctrlKey: false,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBe("paste");
  });

  test("ignores terminal clipboard shortcuts with unsupported modifiers", () => {
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "c",
        code: "KeyC",
        shiftKey: false,
        ctrlKey: true,
        altKey: false,
        metaKey: false,
        isComposing: false,
      }),
    ).toBeNull();
    expect(
      terminalClipboardShortcutActionForKeyEvent({
        key: "c",
        code: "KeyC",
        shiftKey: true,
        ctrlKey: true,
        altKey: true,
        metaKey: false,
        isComposing: false,
      }),
    ).toBeNull();
  });
});

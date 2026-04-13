import { describe, expect, test } from "bun:test";
import { shouldRemapShiftEnterToCtrlJ } from "./terminal-shortcuts";

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

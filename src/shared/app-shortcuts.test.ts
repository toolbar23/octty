import { describe, expect, test } from "bun:test";
import { appShortcutActionForKeyEvent } from "./app-shortcuts";

describe("appShortcutActionForKeyEvent", () => {
  test("maps ctrl-shift pane creation shortcuts", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-shell-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "c",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-codex-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "p",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-pi-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "n",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-nvim-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "j",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-jjui-pane");
    expect(
      appShortcutActionForKeyEvent({
        key: "d",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("open-diff-pane");
  });

  test("keeps existing arrow shortcuts", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowLeft",
        ctrlKey: true,
        shiftKey: false,
        altKey: true,
        metaKey: false,
      }),
    ).toBe("resize-pane-left");
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowRight",
        ctrlKey: true,
        shiftKey: true,
        altKey: true,
        metaKey: false,
      }),
    ).toBe("move-pane-right");
    expect(
      appShortcutActionForKeyEvent({
        key: "ArrowUp",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBe("focus-workspace-up");
  });

  test("ignores unhandled or disallowed chords", () => {
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: false,
        altKey: false,
        metaKey: false,
      }),
    ).toBeNull();
    expect(
      appShortcutActionForKeyEvent({
        key: "s",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: true,
      }),
    ).toBeNull();
    expect(
      appShortcutActionForKeyEvent({
        key: "x",
        ctrlKey: true,
        shiftKey: true,
        altKey: false,
        metaKey: false,
      }),
    ).toBeNull();
  });
});

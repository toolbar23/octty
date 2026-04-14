import { describe, expect, test } from "vitest";
import { readTerminalAppearanceConfig } from "./terminal-config";

describe("readTerminalAppearanceConfig", () => {
  test("uses the octty env vars when set", () => {
    expect(
      readTerminalAppearanceConfig({
        OCTTY_TERMINAL_FONT_FAMILY: '"Iosevka Term", monospace',
        OCTTY_TERMINAL_FONT_SIZE: "15",
      }),
    ).toEqual({
      fontFamily: '"Iosevka Term", monospace',
      fontSize: 15,
    });
  });

  test("falls back to legacy env var names", () => {
    expect(
      readTerminalAppearanceConfig({
        WORKSPACE_ORBIT_TERMINAL_FONT_FAMILY: '"Fira Code", monospace',
        WORKSPACE_ORBIT_TERMINAL_FONT_SIZE: "13",
      }),
    ).toEqual({
      fontFamily: '"Fira Code", monospace',
      fontSize: 13,
    });
  });
});

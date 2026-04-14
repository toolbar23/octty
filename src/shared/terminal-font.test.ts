import { describe, expect, test } from "vitest";
import {
  DEFAULT_TERMINAL_FONT_FAMILY,
  DEFAULT_TERMINAL_FONT_SIZE,
  defaultTerminalAppearanceConfig,
  sanitizeTerminalFontFamily,
  sanitizeTerminalFontSize,
} from "./terminal-font";

describe("terminal font helpers", () => {
  test("falls back to the default family when unset", () => {
    expect(sanitizeTerminalFontFamily(undefined)).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
    expect(sanitizeTerminalFontFamily("   ")).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
  });

  test("keeps configured font families", () => {
    expect(sanitizeTerminalFontFamily('"Iosevka Term", monospace')).toBe(
      '"Iosevka Term", monospace',
    );
  });

  test("uses a clamped integer font size", () => {
    expect(sanitizeTerminalFontSize(undefined)).toBe(DEFAULT_TERMINAL_FONT_SIZE);
    expect(sanitizeTerminalFontSize("15")).toBe(15);
    expect(sanitizeTerminalFontSize("8")).toBe(11);
    expect(sanitizeTerminalFontSize(99)).toBe(24);
  });

  test("uses the shared default stack", () => {
    expect(defaultTerminalAppearanceConfig("linux")).toEqual({
      fontFamily: DEFAULT_TERMINAL_FONT_FAMILY,
      fontSize: DEFAULT_TERMINAL_FONT_SIZE,
    });
  });
});

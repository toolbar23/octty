import {
  defaultTerminalAppearanceConfig,
  sanitizeTerminalFontFamily,
  sanitizeTerminalFontSize,
  type TerminalAppearanceConfig,
} from "../shared/terminal-font";

export function readTerminalAppearanceConfig(
  env: Record<string, string | undefined> = process.env,
): TerminalAppearanceConfig {
  const defaults = defaultTerminalAppearanceConfig(process.platform);
  return {
    fontFamily: sanitizeTerminalFontFamily(
      env.OCTTY_TERMINAL_FONT_FAMILY ?? env.WORKSPACE_ORBIT_TERMINAL_FONT_FAMILY ?? defaults.fontFamily,
    ),
    fontSize: sanitizeTerminalFontSize(
      env.OCTTY_TERMINAL_FONT_SIZE ?? env.WORKSPACE_ORBIT_TERMINAL_FONT_SIZE ?? defaults.fontSize,
    ),
  };
}

export interface TerminalAppearanceConfig {
  fontFamily: string;
  fontSize: number;
}

export const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", monospace';
export const DEFAULT_TERMINAL_FONT_SIZE = 14;
const MIN_TERMINAL_FONT_SIZE = 11;
const MAX_TERMINAL_FONT_SIZE = 24;

export function sanitizeTerminalFontFamily(input: string | undefined | null): string {
  const normalized = input?.trim();
  return normalized ? normalized : DEFAULT_TERMINAL_FONT_FAMILY;
}

export function sanitizeTerminalFontSize(input: string | number | undefined | null): number {
  const parsed =
    typeof input === "number"
      ? input
      : typeof input === "string"
        ? Number.parseInt(input.trim(), 10)
        : Number.NaN;
  if (!Number.isFinite(parsed)) {
    return DEFAULT_TERMINAL_FONT_SIZE;
  }
  return Math.min(MAX_TERMINAL_FONT_SIZE, Math.max(MIN_TERMINAL_FONT_SIZE, Math.round(parsed)));
}

export function defaultTerminalAppearanceConfig(): TerminalAppearanceConfig {
  return {
    fontFamily: DEFAULT_TERMINAL_FONT_FAMILY,
    fontSize: DEFAULT_TERMINAL_FONT_SIZE,
  };
}

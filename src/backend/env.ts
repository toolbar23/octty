const LOADER_ENV_KEYS = [
  "LD_PRELOAD",
  "LD_LIBRARY_PATH",
  "LD_AUDIT",
  "LD_DEBUG",
  "LD_DEBUG_OUTPUT",
  "DYLD_INSERT_LIBRARIES",
  "DYLD_LIBRARY_PATH",
  "TMUX",
  "TMUX_PANE",
] as const;

export function sanitizeChildEnv(
  baseEnv: Record<string, string | undefined> = process.env,
): Record<string, string> {
  const nextEnv: Record<string, string> = {};

  for (const [key, value] of Object.entries(baseEnv)) {
    if (value === undefined || value === null) {
      continue;
    }
    if (LOADER_ENV_KEYS.includes(key as (typeof LOADER_ENV_KEYS)[number])) {
      continue;
    }
    nextEnv[key] = value;
  }

  return nextEnv;
}

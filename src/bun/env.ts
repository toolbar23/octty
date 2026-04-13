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

  /* AGENT-DISCUSSION=  
    [
     { "from":"user", msg:"what is 1+2"   },
     { "from":"agent", msg:"4" },
     { "from":"user", msg:"why does this function exist?" },
     { "from":"agent", msg:"It removes Bun/Electrobun loader variables and tmux-specific variables before spawning child processes, so the terminal sidecar inherits a normal shell environment instead of leaking the parent app runtime or attaching itself to the user's existing tmux session." },
     { "from":"user", msg:"is it possible to remove it?"}"
     { "from":"agent", msg:"You can remove it only if you are sure child processes should inherit the app's loader/runtime variables and any ambient tmux attachment state. In this codebase that would be risky, because spawned shells and tmux-backed sessions are intended to behave like normal user terminals, not nested Electrobun internals." }

    ]
  
 */
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

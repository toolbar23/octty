import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const DEFAULT_APP_DIR_NAME = "octty";

function envValue(
  env: NodeJS.ProcessEnv,
  primaryKey: string,
  legacyKey?: string,
): string | null {
  const value = env[primaryKey] ?? (legacyKey ? env[legacyKey] : undefined);
  if (!value) {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function resolveUserDataDirectory(env: NodeJS.ProcessEnv = process.env): string {
  return (
    envValue(env, "OCTTY_USER_DATA_PATH", "WORKSPACE_ORBIT_USER_DATA_PATH") ??
    join(homedir(), ".local", "share", DEFAULT_APP_DIR_NAME)
  );
}

export function resolveStateDbPath(env: NodeJS.ProcessEnv = process.env): string {
  const preferredPath = join(resolveUserDataDirectory(env), "state.sqlite");
  if (existsSync(preferredPath)) {
    return preferredPath;
  }

  const legacyPaths = [
    join(homedir(), ".local", "share", "octty", "state.sqlite"),
    join(homedir(), ".local", "share", "workspace-orbit", "state.sqlite"),
  ];
  for (const legacyPath of legacyPaths) {
    if (existsSync(legacyPath)) {
      return legacyPath;
    }
  }

  return preferredPath;
}

export function resolveCacheDirectory(env: NodeJS.ProcessEnv = process.env): string {
  return (
    envValue(env, "OCTTY_CACHE_PATH", "WORKSPACE_ORBIT_CACHE_PATH") ??
    join(homedir(), ".cache", DEFAULT_APP_DIR_NAME)
  );
}

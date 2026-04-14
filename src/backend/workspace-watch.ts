const DEFAULT_IGNORED_WORKSPACE_PATH_FRAGMENTS = [
  "/node_modules/",
  "/.git/",
  "/dist/",
  "/artifacts/",
  "/.cache/",
  "/target/",
  "/build/",
  "/out/",
  "/.idea/",
] as const;

const WORKSPACE_WATCH_IGNORE_ENV_KEYS = [
  "OCTTY_WORKSPACE_WATCH_IGNORE",
  "WORKSPACE_ORBIT_WORKSPACE_WATCH_IGNORE",
] as const;

function normalizeFragment(fragment: string): string | null {
  const normalized = fragment.trim().replaceAll("\\", "/");
  if (!normalized) {
    return null;
  }
  if (normalized.includes("/")) {
    return normalized;
  }
  return `/${normalized}/`;
}

export function parseWorkspaceWatchIgnoreFragments(
  env: Record<string, string | undefined> = process.env,
): string[] {
  const fragments: string[] = [...DEFAULT_IGNORED_WORKSPACE_PATH_FRAGMENTS];

  for (const key of WORKSPACE_WATCH_IGNORE_ENV_KEYS) {
    const rawValue = env[key];
    if (!rawValue) {
      continue;
    }

    for (const part of rawValue.split(/[\n,]/)) {
      const normalized = normalizeFragment(part);
      if (normalized) {
        fragments.push(normalized);
      }
    }
  }

  return fragments;
}

export function shouldIgnoreWorkspaceWatchPath(
  pathValue: string,
  env: Record<string, string | undefined> = process.env,
): boolean {
  const normalizedPath = pathValue.replaceAll("\\", "/");
  const normalizedPathWithTrailingSlash = normalizedPath.endsWith("/")
    ? normalizedPath
    : `${normalizedPath}/`;
  const fragments = parseWorkspaceWatchIgnoreFragments(env);
  return fragments.some(
    (fragment) =>
      normalizedPath.includes(fragment) || normalizedPathWithTrailingSlash.includes(fragment),
  );
}

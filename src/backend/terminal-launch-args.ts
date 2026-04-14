import type { TerminalKind } from "../shared/types";

const TERMINAL_ARGS_ENV_PREFIX = "OCTTY_TERMINAL_ARGS_";

type LaunchEnv = Record<string, string | undefined>;

export function terminalArgsEnvKey(kind: TerminalKind): string {
  return `${TERMINAL_ARGS_ENV_PREFIX}${kind.toUpperCase()}`;
}

export function splitShellWords(input: string): string[] {
  const words: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  let escaping = false;

  const pushCurrent = () => {
    if (current.length > 0) {
      words.push(current);
      current = "";
    }
  };

  for (const char of input) {
    if (escaping) {
      current += char;
      escaping = false;
      continue;
    }

    if (quote === "'") {
      if (char === "'") {
        quote = null;
      } else {
        current += char;
      }
      continue;
    }

    if (quote === '"') {
      if (char === '"') {
        quote = null;
      } else if (char === "\\") {
        escaping = true;
      } else {
        current += char;
      }
      continue;
    }

    if (char === "'" || char === '"') {
      quote = char;
      continue;
    }

    if (char === "\\") {
      escaping = true;
      continue;
    }

    if (/\s/.test(char)) {
      pushCurrent();
      continue;
    }

    current += char;
  }

  if (escaping) {
    current += "\\";
  }
  if (quote) {
    throw new Error(`Unterminated ${quote} quote`);
  }

  pushCurrent();
  return words;
}

export function configuredTerminalArgs(
  kind: TerminalKind,
  env: LaunchEnv = process.env,
): string[] {
  const rawValue = env[terminalArgsEnvKey(kind)]?.trim();
  if (!rawValue) {
    return [];
  }

  try {
    return splitShellWords(rawValue);
  } catch (error) {
    console.warn(
      `[terminal-launch] ignoring ${terminalArgsEnvKey(kind)}: ${error instanceof Error ? error.message : String(error)}`,
    );
    return [];
  }
}

export function applyConfiguredTerminalArgs(
  baseArgv: string[],
  kind: TerminalKind,
  env: LaunchEnv = process.env,
): string[] {
  if (baseArgv.length === 0) {
    return [];
  }

  const extraArgs = configuredTerminalArgs(kind, env);
  if (extraArgs.length === 0) {
    return [...baseArgv];
  }

  return [baseArgv[0]!, ...extraArgs, ...baseArgv.slice(1)];
}

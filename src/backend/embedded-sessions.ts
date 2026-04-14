import { readFile, readdir } from "node:fs/promises";
import { homedir } from "node:os";
import { basename, join } from "node:path";
import { defaultTerminalCommand } from "../shared/terminal-kind";
import type { EmbeddedSessionRef, TerminalKind } from "../shared/types";
import { applyConfiguredTerminalArgs, configuredTerminalArgs } from "./terminal-launch-args";

type TerminalLaunchSpec = {
  argv: string[];
  displayCommand: string;
};

type DetectEmbeddedSessionArgs = {
  cwd: string;
  launchedAt?: number;
  correlationId?: string | null;
};

type EmbeddedSessionProvider = {
  providerName: string;
  kind: TerminalKind;
  buildLaunch(
    embeddedSession: EmbeddedSessionRef | null,
    correlationId?: string | null,
    env?: Record<string, string | undefined>,
  ): TerminalLaunchSpec;
  detectExternalSession(args: DetectEmbeddedSessionArgs): Promise<EmbeddedSessionRef | null>;
};

type CodexSessionMeta = {
  type: "session_meta";
  payload?: {
    id?: unknown;
    cwd?: unknown;
    timestamp?: unknown;
  };
};

const CODEX_SESSION_LOOKBACK_MS = 10 * 60_000;
const CODEX_SESSION_EARLY_SKEW_MS = 60_000;
const EMBEDDED_SESSION_CORRELATION_PREFIX = "octty-embedded-session:";

function joinShellWords(argv: string[]): string {
  return argv
    .map((value) =>
      /^[A-Za-z0-9_./:-]+$/.test(value) ? value : `'${value.replace(/'/g, `'\"'\"'`)}'`,
    )
    .join(" ");
}

function dateDirFor(timestamp: number): string {
  const date = new Date(timestamp);
  const year = String(date.getFullYear());
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return join(year, month, day);
}

function timestampFromCorrelationId(correlationId: string | null | undefined): number | null {
  if (!correlationId?.startsWith(EMBEDDED_SESSION_CORRELATION_PREFIX)) {
    return null;
  }

  const remainder = correlationId.slice(EMBEDDED_SESSION_CORRELATION_PREFIX.length);
  const separatorIndex = remainder.indexOf(":");
  const timestampText = separatorIndex === -1 ? remainder : remainder.slice(0, separatorIndex);
  const parsed = Number.parseInt(timestampText, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

export function createEmbeddedSessionCorrelationId(
  launchedAt: number,
  sessionId: string,
): string {
  return `${EMBEDDED_SESSION_CORRELATION_PREFIX}${launchedAt}:${sessionId}`;
}

function codexCorrelationPrompt(correlationId: string): string {
  return [
    "Session bookkeeping marker:",
    correlationId,
    'Reply with exactly "Ready."',
  ].join(" ");
}

async function detectCodexSessionFromRoot(
  args: DetectEmbeddedSessionArgs & { sessionsRoot: string },
): Promise<EmbeddedSessionRef | null> {
  const correlationTimestamp = timestampFromCorrelationId(args.correlationId);
  const searchBaseTimestamp = correlationTimestamp ?? args.launchedAt ?? Date.now();
  const candidateDirs = Array.from(
    new Set([
      dateDirFor(searchBaseTimestamp - 24 * 60 * 60_000),
      dateDirFor(searchBaseTimestamp),
      dateDirFor(searchBaseTimestamp + 24 * 60 * 60_000),
    ]),
  );

  const candidates: Array<{ id: string; timestamp: number; label: string }> = [];
  for (const relativeDir of candidateDirs) {
    const absoluteDir = join(args.sessionsRoot, relativeDir);
    let fileNames: string[];
    try {
      fileNames = (await readdir(absoluteDir)).filter((name) => name.endsWith(".jsonl"));
    } catch {
      continue;
    }

    for (const fileName of fileNames.sort().reverse().slice(0, 40)) {
      try {
        const fileContents = await readFile(join(absoluteDir, fileName), "utf8");
        const firstLine = fileContents.split("\n")[0]?.trim();
        if (!firstLine) {
          continue;
        }

        const parsed = JSON.parse(firstLine) as CodexSessionMeta;
        if (parsed.type !== "session_meta") {
          continue;
        }

        const sessionId = typeof parsed.payload?.id === "string" ? parsed.payload.id : null;
        const cwd = typeof parsed.payload?.cwd === "string" ? parsed.payload.cwd : null;
        const timestampValue =
          typeof parsed.payload?.timestamp === "string" ? Date.parse(parsed.payload.timestamp) : NaN;
        if (!sessionId || !cwd || cwd !== args.cwd || !Number.isFinite(timestampValue)) {
          continue;
        }

        if (args.correlationId && !fileContents.includes(args.correlationId)) {
          continue;
        }
        if (
          !args.correlationId &&
          args.launchedAt !== undefined &&
          (
            timestampValue < args.launchedAt - CODEX_SESSION_EARLY_SKEW_MS ||
            timestampValue > args.launchedAt + CODEX_SESSION_LOOKBACK_MS
          )
        ) {
          continue;
        }

        candidates.push({
          id: sessionId,
          timestamp: timestampValue,
          label: basename(fileName, ".jsonl"),
        });
      } catch {
        continue;
      }
    }
  }

  if (candidates.length === 0) {
    return null;
  }

  candidates.sort((left, right) => {
    const leftDistance = Math.abs(left.timestamp - searchBaseTimestamp);
    const rightDistance = Math.abs(right.timestamp - searchBaseTimestamp);
    return leftDistance - rightDistance || right.timestamp - left.timestamp;
  });

  const match = candidates[0]!;
  return {
    provider: "codex",
    id: match.id,
    label: match.label,
    detectedAt: Date.now(),
  };
}

const codexProvider: EmbeddedSessionProvider = {
  providerName: "codex",
  kind: "codex",
  buildLaunch(embeddedSession, correlationId, env = process.env) {
    const baseArgv = embeddedSession
      ? ["codex", "resume", embeddedSession.id]
      : correlationId
        ? ["codex", codexCorrelationPrompt(correlationId)]
        : ["codex"];
    const argv = applyConfiguredTerminalArgs(baseArgv, "codex", env);
    const displayArgv = embeddedSession ? argv : ["codex", ...configuredTerminalArgs("codex", env)];
    return {
      argv,
      displayCommand: joinShellWords(displayArgv),
    };
  },
  detectExternalSession(args) {
    return detectCodexSessionFromRoot({
      ...args,
      sessionsRoot: join(homedir(), ".codex", "sessions"),
    });
  },
};

const EMBEDDED_SESSION_PROVIDERS = new Map<TerminalKind, EmbeddedSessionProvider>([
  [codexProvider.kind, codexProvider],
]);

export function getEmbeddedSessionProvider(kind: TerminalKind): EmbeddedSessionProvider | null {
  return EMBEDDED_SESSION_PROVIDERS.get(kind) ?? null;
}

export function buildTerminalLaunch(
  kind: TerminalKind,
  embeddedSession: EmbeddedSessionRef | null,
  correlationId: string | null = null,
  env: Record<string, string | undefined> = process.env,
): TerminalLaunchSpec {
  const provider = getEmbeddedSessionProvider(kind);
  if (provider) {
    return provider.buildLaunch(embeddedSession, correlationId, env);
  }

  const command = defaultTerminalCommand(kind);
  const argv = applyConfiguredTerminalArgs([command], kind, env);
  return {
    argv,
    displayCommand: joinShellWords(argv),
  };
}

export async function detectEmbeddedSession(
  kind: TerminalKind,
  args: DetectEmbeddedSessionArgs,
): Promise<EmbeddedSessionRef | null> {
  const provider = getEmbeddedSessionProvider(kind);
  if (!provider) {
    return null;
  }

  return provider.detectExternalSession(args);
}

export const __testOnly = {
  detectCodexSessionFromRoot,
  codexCorrelationPrompt,
  timestampFromCorrelationId,
};

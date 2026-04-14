import { fork, spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdirSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import type { SessionSnapshot, TerminalKind } from "../shared/types";
import { resolveCacheDirectory } from "./app-paths";
import { sanitizeChildEnv } from "./env";
import { defaultTerminalCommand, isAgentTerminalKind, normalizeTerminalKind } from "../shared/terminal-kind";

interface SidecarEnvelope {
  type: "create" | "write" | "resize" | "kill";
  sessionId: string;
  command?: string;
  args?: string[];
  cwd?: string;
  cols?: number;
  rows?: number;
  data?: string;
}

interface LiveSession extends SessionSnapshot {
  commandArgs: string[];
  tmuxSessionName: string;
}

const MAX_SESSION_BUFFER = 64_000;
const OCTTY_TMUX_SOCKET = "octty";
const OCTTY_TMUX_CONFIG = `# Octty owns the outer UI chrome, so tmux should stay invisible and inert.
set -g prefix None
set -g prefix2 None
set -g status off
set -g pane-border-status off
set -g mouse off
unbind-key -a
unbind-key -a -T root
`;

type SidecarListener = (message: {
  type: "ready" | "output" | "exit" | "error";
  sessionId?: string;
  data?: string;
  exitCode?: number;
  message?: string;
}) => void;

export function sidecarPathCandidates(
  sourceRoot: string | null,
  cwd: string,
  moduleUrl: string = import.meta.url,
): string[] {
  return [
    sourceRoot ? resolve(sourceRoot, "src/pty-host/index.mjs") : null,
    sourceRoot ? resolve(sourceRoot, "runtime/pty-host/index.mjs") : null,
    sourceRoot ? resolve(sourceRoot, "build/electron/runtime/pty-host/index.mjs") : null,
    resolve(cwd, "src/pty-host/index.mjs"),
    resolve(cwd, "build/electron/runtime/pty-host/index.mjs"),
    resolve(cwd, "runtime/pty-host/index.mjs"),
    fileURLToPath(new URL("./runtime/pty-host/index.mjs", moduleUrl)),
  ].filter((candidate): candidate is string => Boolean(candidate));
}

export function resolveSidecarWorkingDirectory(
  sourceRoot: string | null,
  fallbackCwd: string = process.cwd(),
): string {
  if (sourceRoot) {
    try {
      if (statSync(sourceRoot).isDirectory()) {
        return sourceRoot;
      }
    } catch {
      // Ignore missing or unreadable roots and fall back to a known-good cwd.
    }
  }
  return fallbackCwd;
}

function resolveSidecarPath(): string {
  const sourceRoot = process.env.OCTTY_SOURCE_ROOT || process.env.WORKSPACE_ORBIT_SOURCE_ROOT;
  const candidates = sidecarPathCandidates(sourceRoot ?? null, process.cwd());

  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return realpathSync(candidate);
    }
  }

  throw new Error("Could not locate PTY sidecar entrypoint");
}

function shellQuote(value: string): string {
  return `'${value.replace(/'/g, `'\"'\"'`)}'`;
}

function shellExecTarget(command: string): string {
  return /^[A-Za-z0-9_./:-]+$/.test(command) ? command : shellQuote(command);
}

function shellJoinWords(argv: string[]): string {
  return argv.map((value) => shellExecTarget(value)).join(" ");
}

export function shellCommandFor(
  kind: TerminalKind,
  shellPath: string = process.env.SHELL || "/bin/bash",
  toolArgv?: string[],
): { command: string; args: string[] } {
  switch (normalizeTerminalKind(kind)) {
    case "shell":
      return {
        command: shellPath,
        args: ["-l"],
      };
    case "codex":
    case "pi":
    case "nvim":
    case "jjui":
      return {
        command: shellPath,
        args: ["-lic", `exec ${shellJoinWords(toolArgv ?? [defaultTerminalCommand(kind)])}`],
      };
  }
}

function tmuxSessionNameFor(sessionId: string): string {
  return `octty-${sessionId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

function legacyTmuxSessionNameFor(sessionId: string): string {
  return `workspace-orbit-${sessionId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

function resolveTmuxConfigPath(): string {
  const configDir = resolveCacheDirectory();
  const configPath = join(configDir, "tmux.conf");
  mkdirSync(configDir, { recursive: true });
  writeFileSync(configPath, OCTTY_TMUX_CONFIG);
  return configPath;
}

type TmuxTarget = {
  mode: "octty" | "legacy-default";
  sessionName: string;
};

export class PtySidecar {
  private readonly sessions = new Map<string, LiveSession>();
  private readonly listeners = new Set<SidecarListener>();
  private readonly proc: ChildProcess;
  private readonly childEnv = sanitizeChildEnv();
  private readonly tmuxConfigPath = resolveTmuxConfigPath();

  constructor() {
    const sourceRoot = process.env.OCTTY_SOURCE_ROOT || process.env.WORKSPACE_ORBIT_SOURCE_ROOT;
    const proc = fork(resolveSidecarPath(), {
      cwd: resolveSidecarWorkingDirectory(sourceRoot ?? null),
      env: this.childEnv,
      stdio: ["pipe", "pipe", "pipe", "ipc"],
      silent: true,
    });
    if (!proc.stdin || !proc.stdout || !proc.stderr) {
      throw new Error("PTY sidecar stdio was not available");
    }

    this.proc = proc;
    this.consumeStdout(proc.stdout);
    this.consumeStderr(proc.stderr);
  }

  private consumeStdout(stream: NodeJS.ReadableStream): void {
    const reader = createInterface({ input: stream });
    reader.on("line", (line) => {
      if (!line.trim()) {
        return;
      }
      const message = JSON.parse(line) as Parameters<SidecarListener>[0];
      if (message.type === "exit" && message.sessionId) {
        const session = this.sessions.get(message.sessionId);
        if (session) {
          session.state = "stopped";
          session.exitCode = message.exitCode ?? null;
        }
      }
      this.listeners.forEach((listener) => listener(message));
    });
  }

  private consumeStderr(stream: NodeJS.ReadableStream): void {
    const chunks: Buffer[] = [];
    stream.on("data", (chunk: Buffer | string) => {
      chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
    });
    stream.on("end", () => {
      const text = Buffer.concat(chunks).toString("utf8").trim();
      if (text) {
        console.error("[pty-sidecar]", text);
      }
    });
  }

  private send(message: SidecarEnvelope): void {
    if (!this.proc.stdin) {
      throw new Error("PTY sidecar stdin is not available");
    }
    this.proc.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private tmuxCommandArgs(mode: TmuxTarget["mode"], args: string[]): string[] {
    if (mode === "octty") {
      return ["-L", OCTTY_TMUX_SOCKET, "-f", this.tmuxConfigPath, ...args];
    }
    return args;
  }

  private runTmuxCapture(
    args: string[],
    mode: TmuxTarget["mode"] = "octty",
  ): { success: boolean; stdout: string; stderr: string } {
    const result = spawnSync("tmux", this.tmuxCommandArgs(mode, args), {
      cwd: process.cwd(),
      env: this.childEnv,
      encoding: "utf8",
    });
    return {
      success: result.status === 0,
      stdout: result.stdout ?? "",
      stderr: result.stderr ?? "",
    };
  }

  private spawnTmux(args: string[], mode: TmuxTarget["mode"] = "octty"): void {
    const proc = spawn("tmux", this.tmuxCommandArgs(mode, args), {
      cwd: process.cwd(),
      env: this.childEnv,
      stdio: "ignore",
    });
    proc.unref();
  }

  private resolveTmuxTarget(sessionId: string): TmuxTarget {
    const candidates: TmuxTarget[] = [
      { mode: "octty", sessionName: tmuxSessionNameFor(sessionId) },
      { mode: "octty", sessionName: legacyTmuxSessionNameFor(sessionId) },
      { mode: "legacy-default", sessionName: legacyTmuxSessionNameFor(sessionId) },
      { mode: "legacy-default", sessionName: tmuxSessionNameFor(sessionId) },
    ];
    for (const candidate of candidates) {
      const result = this.runTmuxCapture(["has-session", "-t", candidate.sessionName], candidate.mode);
      if (result.success) {
        return candidate;
      }
    }
    return candidates[0]!;
  }

  onMessage(listener: SidecarListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  createSession(input: {
    sessionId: string;
    workspaceId: string;
    paneId: string;
    kind: TerminalKind;
    cwd: string;
    cols: number;
    rows: number;
    launchArgv?: string[];
    displayCommand?: string;
    embeddedSession: SessionSnapshot["embeddedSession"];
    embeddedSessionCorrelationId: SessionSnapshot["embeddedSessionCorrelationId"];
    agentAttentionState?: SessionSnapshot["agentAttentionState"];
  }): LiveSession {
    const tmuxTarget = this.resolveTmuxTarget(input.sessionId);
    const tmuxSessionName = tmuxTarget.sessionName;
    const { command, args } = shellCommandFor(input.kind, undefined, input.launchArgv);
    const session: LiveSession = {
      id: input.sessionId,
      workspaceId: input.workspaceId,
      paneId: input.paneId,
      kind: normalizeTerminalKind(input.kind),
      cwd: input.cwd,
      command: input.displayCommand || command,
      commandArgs: args,
      buffer: "",
      state: "live",
      exitCode: null,
      embeddedSession: input.embeddedSession,
      embeddedSessionCorrelationId: input.embeddedSessionCorrelationId,
      agentAttentionState:
        input.agentAttentionState ??
        (isAgentTerminalKind(normalizeTerminalKind(input.kind)) ? "idle-seen" : null),
      tmuxSessionName,
    };

    this.sessions.set(session.id, session);
    this.send({
      type: "create",
      sessionId: session.id,
      command: "tmux",
      args: this.tmuxCommandArgs(tmuxTarget.mode, [
        "new-session",
        "-A",
        "-s",
        tmuxSessionName,
        "-c",
        input.cwd,
        command,
        ...args,
      ]),
      cwd: input.cwd,
      cols: input.cols,
      rows: input.rows,
    });

    return session;
  }

  getSession(sessionId: string): LiveSession | undefined {
    return this.sessions.get(sessionId);
  }

  captureScreen(sessionId: string): string {
    if (!sessionId) {
      return "";
    }

    const tmuxTarget = this.resolveTmuxTarget(sessionId);
    const result = this.runTmuxCapture(
      ["capture-pane", "-p", "-e", "-t", `${tmuxTarget.sessionName}:0.0`],
      tmuxTarget.mode,
    );
    if (!result.success) {
      return this.sessions.get(sessionId)?.buffer ?? "";
    }
    return result.stdout;
  }

  appendOutput(sessionId: string, data: string): void {
    const session = this.sessions.get(sessionId);
    if (!session) {
      return;
    }

    session.buffer = `${session.buffer}${data}`.slice(-MAX_SESSION_BUFFER);
  }

  write(sessionId: string, data: string): void {
    this.send({
      type: "write",
      sessionId,
      data,
    });
  }

  resize(sessionId: string, cols: number, rows: number): void {
    this.send({
      type: "resize",
      sessionId,
      cols,
      rows,
    });
  }

  kill(sessionId: string): void {
    if (!sessionId) {
      return;
    }
    if (this.sessions.has(sessionId)) {
      this.send({
        type: "kill",
        sessionId,
      });
      this.sessions.delete(sessionId);
    }
    for (const tmuxTarget of [
      { mode: "octty", sessionName: tmuxSessionNameFor(sessionId) },
      { mode: "octty", sessionName: legacyTmuxSessionNameFor(sessionId) },
      { mode: "legacy-default", sessionName: legacyTmuxSessionNameFor(sessionId) },
      { mode: "legacy-default", sessionName: tmuxSessionNameFor(sessionId) },
    ] as const) {
      this.spawnTmux(["kill-session", "-t", tmuxTarget.sessionName], tmuxTarget.mode);
    }
  }

  detach(sessionId: string): void {
    if (!sessionId || !this.sessions.has(sessionId)) {
      return;
    }
    this.send({
      type: "kill",
      sessionId,
    });
    this.sessions.delete(sessionId);
  }

  listSessions(): LiveSession[] {
    return Array.from(this.sessions.values());
  }

  dispose(): void {
    for (const sessionId of Array.from(this.sessions.keys())) {
      this.detach(sessionId);
    }
    this.proc.stdin?.end();
    this.proc.kill();
  }
}

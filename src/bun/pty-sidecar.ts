import { realpathSync } from "node:fs";
import { existsSync } from "node:fs";
import { resolve } from "node:path";
import type { SessionSnapshot, TerminalKind } from "../shared/types";
import { sanitizeChildEnv } from "./env";
import { defaultTerminalCommand, normalizeTerminalKind } from "../shared/terminal-kind";

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

type SidecarListener = (message: {
  type: "ready" | "output" | "exit" | "error";
  sessionId?: string;
  data?: string;
  exitCode?: number;
  message?: string;
}) => void;

function resolveSidecarPath(): string {
  const sourceRoot = process.env.OCTTY_SOURCE_ROOT || process.env.WORKSPACE_ORBIT_SOURCE_ROOT;
  const candidates = [
    sourceRoot ? resolve(sourceRoot, "src/pty-host/index.mjs") : null,
    resolve(process.cwd(), "src/pty-host/index.mjs"),
    resolve(process.cwd(), "runtime/pty-host/index.mjs"),
    new URL("../runtime/pty-host/index.mjs", import.meta.url).pathname,
  ];

  for (const candidate of candidates) {
    if (!candidate) {
      continue;
    }
    if (existsSync(candidate)) {
      return realpathSync(candidate);
    }
  }

  throw new Error("Could not locate PTY sidecar entrypoint");
}

function shellCommandFor(kind: TerminalKind): { command: string; args: string[] } {
  switch (normalizeTerminalKind(kind)) {
    case "shell":
      return {
        command: process.env.SHELL || "/bin/bash",
        args: ["-l"],
      };
    case "codex":
    case "pi":
    case "nvim":
    case "jjui":
      return {
        command: defaultTerminalCommand(kind),
        args: [],
      };
  }
}

function tmuxSessionNameFor(sessionId: string): string {
  return `octty-${sessionId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

function legacyTmuxSessionNameFor(sessionId: string): string {
  return `workspace-orbit-${sessionId.replace(/[^A-Za-z0-9_-]/g, "_")}`;
}

export class PtySidecar {
  private readonly sessions = new Map<string, LiveSession>();
  private readonly listeners = new Set<SidecarListener>();
  private readonly proc: ReturnType<typeof Bun.spawn>;
  private readonly stdin: {
    write: (chunk: string) => unknown;
    end?: () => unknown;
  };
  private readonly childEnv = sanitizeChildEnv();

  constructor() {
    const sourceRoot = process.env.OCTTY_SOURCE_ROOT || process.env.WORKSPACE_ORBIT_SOURCE_ROOT;
    const proc = Bun.spawn({
      cmd: ["node", resolveSidecarPath()],
      cwd: sourceRoot || process.cwd(),
      stdin: "pipe",
      stdout: "pipe",
      stderr: "pipe",
      env: this.childEnv,
    });

    this.proc = proc;
    this.stdin = proc.stdin as unknown as {
      write: (chunk: string) => unknown;
      end?: () => unknown;
    };
    this.consumeStdout(proc.stdout);
    this.consumeStderr(proc.stderr);
  }

  private async consumeStdout(stream: ReadableStream<Uint8Array>): Promise<void> {
    const reader = stream.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        return;
      }

      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split("\n");
      buffer = lines.pop() ?? "";

      for (const line of lines) {
        if (!line.trim()) {
          continue;
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
      }
    }
  }

  private async consumeStderr(stream: ReadableStream<Uint8Array>): Promise<void> {
    const text = await new Response(stream).text();
    if (text.trim()) {
      console.error("[pty-sidecar]", text.trim());
    }
  }

  private send(message: SidecarEnvelope): void {
    this.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private runTmuxCapture(args: string[]): { success: boolean; stdout: string; stderr: string } {
    const result = Bun.spawnSync({
      cmd: ["tmux", ...args],
      cwd: process.cwd(),
      env: this.childEnv,
      stdout: "pipe",
      stderr: "pipe",
    });
    const decoder = new TextDecoder();
    return {
      success: result.exitCode === 0,
      stdout: decoder.decode(result.stdout),
      stderr: decoder.decode(result.stderr),
    };
  }

  private spawnTmux(args: string[]): void {
    const proc = Bun.spawn({
      cmd: ["tmux", ...args],
      cwd: process.cwd(),
      env: this.childEnv,
      stdout: "ignore",
      stderr: "ignore",
    });
    void proc.exited.catch(() => {});
  }

  private resolveTmuxSessionName(sessionId: string): string {
    const names = [tmuxSessionNameFor(sessionId), legacyTmuxSessionNameFor(sessionId)];
    for (const name of names) {
      const result = this.runTmuxCapture(["has-session", "-t", name]);
      if (result.success) {
        return name;
      }
    }
    return names[0]!;
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
  }): LiveSession {
    const tmuxSessionName = this.resolveTmuxSessionName(input.sessionId);
    const { command, args } = shellCommandFor(input.kind);
    const session: LiveSession = {
      id: input.sessionId,
      workspaceId: input.workspaceId,
      paneId: input.paneId,
      kind: normalizeTerminalKind(input.kind),
      cwd: input.cwd,
      command,
      commandArgs: args,
      buffer: "",
      state: "live",
      exitCode: null,
      tmuxSessionName,
    };

    this.sessions.set(session.id, session);
    this.send({
      type: "create",
      sessionId: session.id,
      command: "tmux",
      args: ["new-session", "-A", "-s", tmuxSessionName, "-c", input.cwd, command, ...args],
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

    const result = this.runTmuxCapture([
      "capture-pane",
      "-p",
      "-e",
      "-t",
      `${this.resolveTmuxSessionName(sessionId)}:0.0`,
    ]);
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
    for (const tmuxSessionName of [
      tmuxSessionNameFor(sessionId),
      legacyTmuxSessionNameFor(sessionId),
    ]) {
      this.spawnTmux(["kill-session", "-t", tmuxSessionName]);
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
    this.stdin.end?.();
    this.proc.kill();
  }
}

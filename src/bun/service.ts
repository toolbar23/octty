import { basename, join } from "node:path";
import { homedir } from "node:os";
import { existsSync } from "node:fs";
import {
  mkdir,
  readFile,
  readdir,
  realpath,
  stat,
  unlink,
  writeFile,
} from "node:fs/promises";
import chokidar from "chokidar";
import { Utils } from "electrobun/bun";
import {
  addPane,
  createDefaultSnapshot,
  sanitizeSnapshot,
  updatePane,
} from "../shared/layout";
import type {
  BootstrapPayload,
  CreateWorkspacePayload,
  NoteRecord,
  PaneState,
  ProjectRootRecord,
  SessionSnapshot,
  TerminalPanePayload,
  TerminalCreateRequest,
  WorkspaceDetail,
  WorkspaceSnapshot,
  WorkspaceSummary,
} from "../shared/types";
import { hasRecordedWorkspacePath } from "../shared/types";
import { AppDatabase } from "./db";
import {
  buildTerminalLaunch,
  createEmbeddedSessionCorrelationId,
  detectEmbeddedSession,
  getEmbeddedSessionProvider,
} from "./embedded-sessions";
import {
  createWorkspace as jjCreateWorkspace,
  discoverWorkspaces,
  forgetWorkspace as jjForgetWorkspace,
  readWorkspaceStatus,
  resolveRepoRoot,
} from "./jj";
import { PtySidecar } from "./pty-sidecar";
import { restoreTerminalPanePayload } from "./terminal-restore";
import { isAgentTerminalKind, normalizeTerminalKind } from "../shared/terminal-kind";

type ClientSink = (message: { type: string; payload: unknown }) => void;

interface WorkspaceRuntime {
  workspaceId: string;
  lastOpenedAt: number;
  sessionIds: Set<string>;
}

const DEBUG_TERMINAL_IO =
  process.env.OCTTY_DEBUG_TERMINAL === "1" || process.env.WORKSPACE_ORBIT_DEBUG_TERMINAL === "1";

function defaultDbPath(): string {
  const octtyPath = join(homedir(), ".local", "share", "octty", "state.sqlite");
  const legacyPath = join(homedir(), ".local", "share", "workspace-orbit", "state.sqlite");
  return existsSync(legacyPath) ? legacyPath : octtyPath;
}

function now(): number {
  return Date.now();
}

function formatTerminalChunk(data: string, limit = 120): string {
  const serialized = JSON.stringify(data);
  return serialized.length > limit ? `${serialized.slice(0, limit)}...` : serialized;
}

function slugifyNote(fileName: string): string {
  const trimmed = fileName.trim().replace(/\.note\.md$/i, "");
  const normalized = trimmed
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return normalized || "note";
}

function extractNoteTitle(fileName: string, body: string): string {
  const lines = body.split("\n");
  const heading = lines.find((line) => line.trim().startsWith("#"));
  if (heading) {
    return heading.replace(/^#+\s*/, "").trim();
  }
  return fileName.replace(/\.note\.md$/i, "");
}

function shouldIgnoreWorkspaceWatchPath(pathValue: string): boolean {
  const normalized = pathValue.replaceAll("\\", "/");
  return (
    normalized.includes("/node_modules/") ||
    normalized.includes("/.git/") ||
    normalized.includes("/.jj/") ||
    normalized.includes("/dist/") ||
    normalized.includes("/artifacts/") ||
    normalized.includes("/.cache/")
  );
}

export class WorkspaceService {
  private readonly db: AppDatabase;
  private readonly ptySidecar: PtySidecar;
  private readonly clients = new Set<ClientSink>();
  private readonly runtimes = new Map<string, WorkspaceRuntime>();
  private readonly watchers = new Map<string, ReturnType<typeof chokidar.watch>>();
  private readonly refreshTimers = new Map<string, Timer>();
  private readonly sessionPersistTimers = new Map<string, Timer>();
  private readonly embeddedSessionDetectTimers = new Map<string, Timer>();

  private logTerminal(
    message: string,
    details?: Record<string, unknown> | (() => Record<string, unknown>),
  ): void {
    if (!DEBUG_TERMINAL_IO) {
      return;
    }

    const resolvedDetails = typeof details === "function" ? details() : details;
    console.log("[terminal]", message, resolvedDetails ?? {});
  }

  constructor(private readonly dbPath = defaultDbPath()) {
    this.db = new AppDatabase(dbPath);
    this.ptySidecar = new PtySidecar();
    this.ptySidecar.onMessage((message) => {
      if (message.type === "output" && message.sessionId && message.data) {
        const sessionId = message.sessionId;
        const data = message.data;
        this.logTerminal("output", () => ({
          sessionId,
          chunk: formatTerminalChunk(data),
        }));
        this.ptySidecar.appendOutput(sessionId, data);
        this.scheduleSessionPersist(sessionId);
        this.broadcast({
          type: "terminal-output",
          payload: {
            sessionId,
            data,
          },
        });
      }

      if (message.type === "exit" && message.sessionId) {
        this.logTerminal("exit", {
          sessionId: message.sessionId,
          exitCode: message.exitCode ?? null,
        });
        const persistTimer = this.sessionPersistTimers.get(message.sessionId);
        if (persistTimer) {
          clearTimeout(persistTimer);
          this.sessionPersistTimers.delete(message.sessionId);
        }
        const session = this.ptySidecar.getSession(message.sessionId);
        if (session) {
          session.state = "stopped";
          session.exitCode = message.exitCode ?? null;
          this.db.saveSessionState(session);
          this.syncTerminalPaneSnapshot(session.workspaceId, session.paneId, (payload) => ({
            ...payload,
            sessionId: null,
            sessionState: "stopped",
            autoStart: false,
            exitCode: session.exitCode,
            cwd: session.cwd,
            command: session.command,
          }));
          this.updateActiveAgentCounts(session.workspaceId);
        }
        this.broadcast({
          type: "terminal-exit",
          payload: {
            sessionId: message.sessionId,
            exitCode: message.exitCode ?? null,
          },
        });
      }
    });
  }

  async init(): Promise<void> {
    const roots = this.db.listProjectRoots();
    for (const root of roots) {
      await this.syncProjectRoot(root.id, root.rootPath);
    }
  }

  dispose(): void {
    for (const timer of this.refreshTimers.values()) {
      clearTimeout(timer);
    }
    for (const timer of this.sessionPersistTimers.values()) {
      clearTimeout(timer);
    }
    for (const timer of this.embeddedSessionDetectTimers.values()) {
      clearTimeout(timer);
    }
    for (const watcher of this.watchers.values()) {
      void watcher.close();
    }
    this.watchers.clear();
    this.embeddedSessionDetectTimers.clear();
    this.ptySidecar.dispose();
    this.db.close();
  }

  addClient(client: ClientSink): () => void {
    this.clients.add(client);
    return () => {
      this.clients.delete(client);
    };
  }

  private broadcast(message: { type: string; payload: unknown }): void {
    for (const client of this.clients) {
      client(message);
    }
  }

  getBootstrap(): BootstrapPayload {
    return {
      projectRoots: this.db.listProjectRoots(),
      workspaces: this.db.listWorkspaces(),
    };
  }

  async pickDirectory(startingFolder?: string): Promise<string | null> {
    const paths = await Utils.openFileDialog({
      startingFolder: startingFolder || homedir(),
      canChooseFiles: false,
      canChooseDirectory: true,
      allowsMultipleSelection: false,
    });

    return paths[0] || null;
  }

  async addProjectRoot(inputPath: string): Promise<ProjectRootRecord> {
    const rootPath = await resolveRepoRoot(inputPath);
    const existing = this.db
      .listProjectRoots()
      .find((root) => root.rootPath === rootPath);
    if (existing) {
      return existing;
    }

    const record: ProjectRootRecord = {
      id: globalThis.crypto.randomUUID(),
      rootPath,
      label: basename(rootPath),
      createdAt: now(),
      updatedAt: now(),
    };

    this.db.upsertProjectRoot(record);
    await this.syncProjectRoot(record.id, record.rootPath);
    this.broadcast({
      type: "nav-updated",
      payload: this.getBootstrap(),
    });
    return record;
  }

  async removeProjectRoot(rootId: string): Promise<void> {
    const workspaces = this.db.listWorkspaces().filter((workspace) => workspace.rootId === rootId);
    for (const workspace of workspaces) {
      await this.closeWorkspaceRuntime(workspace.id);
      const watcher = this.watchers.get(workspace.id);
      if (watcher) {
        await watcher.close();
        this.watchers.delete(workspace.id);
      }
    }

    this.db.deleteProjectRoot(rootId);
    this.broadcast({
      type: "nav-updated",
      payload: this.getBootstrap(),
    });
  }

  async syncProjectRoot(rootId: string, rootPath: string): Promise<void> {
    const current = this.db.listProjectRoots().find((root) => root.id === rootId);
    const discovered = await discoverWorkspaces(rootPath);
    const seenIds: string[] = [];

    for (const workspace of discovered) {
      const existing = this.db.listWorkspaces().find((item) => item.id === workspace.id);
      const summary: WorkspaceSummary = {
        id: workspace.id,
        rootId,
        rootPath,
        projectLabel: current?.label ?? basename(rootPath),
        workspaceName: workspace.workspaceName,
        workspacePath: workspace.workspacePath,
        dirty: existing?.dirty ?? false,
        bookmarks: existing?.bookmarks ?? [],
        unreadNotes: existing?.unreadNotes ?? 0,
        activeAgentCount: existing?.activeAgentCount ?? 0,
        recentActivityAt: existing?.recentActivityAt ?? 0,
        diffText: existing?.diffText ?? "",
        createdAt: existing?.createdAt ?? now(),
        updatedAt: now(),
        lastOpenedAt: existing?.lastOpenedAt ?? 0,
      };
      this.db.upsertWorkspace(summary);
      seenIds.push(summary.id);
      this.ensureWatcher(summary);
      try {
        await this.refreshWorkspace(summary.id);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`[workspace-sync] ${summary.workspaceName}: ${message}`);
      }
    }

    this.db.deleteWorkspacesMissing(rootId, seenIds);
  }

  private ensureWatcher(workspace: WorkspaceSummary): void {
    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      return;
    }

    if (this.watchers.has(workspace.id)) {
      return;
    }

    const watcher = chokidar.watch(workspace.workspacePath, {
      ignoreInitial: true,
      ignored: (pathValue) => shouldIgnoreWorkspaceWatchPath(String(pathValue)),
    });

    const schedule = () => {
      this.scheduleRefresh(workspace.id);
    };
    const eventWatcher = watcher as unknown as {
      on: (event: string, listener: (...args: unknown[]) => void) => unknown;
    };

    eventWatcher.on("add", schedule);
    eventWatcher.on("change", schedule);
    eventWatcher.on("unlink", schedule);
    eventWatcher.on("addDir", schedule);
    eventWatcher.on("unlinkDir", schedule);
    eventWatcher.on("error", (error: unknown) => {
      const message = error instanceof Error ? error.message : String(error);
      console.warn(`[workspace-watch] ${workspace.workspaceName}: ${message}`);
    });

    this.watchers.set(workspace.id, watcher);
  }

  private scheduleRefresh(workspaceId: string): void {
    const existing = this.refreshTimers.get(workspaceId);
    if (existing) {
      clearTimeout(existing);
    }

    const timer = setTimeout(() => {
      void this.refreshWorkspace(workspaceId).catch((error) => {
        const message = error instanceof Error ? error.message : String(error);
        console.warn(`[workspace-refresh] ${workspaceId}: ${message}`);
      });
    }, 180);
    this.refreshTimers.set(workspaceId, timer);
  }

  private scheduleSessionPersist(sessionId: string): void {
    const existing = this.sessionPersistTimers.get(sessionId);
    if (existing) {
      clearTimeout(existing);
    }

    const timer = setTimeout(() => {
      const session = this.ptySidecar.getSession(sessionId);
      if (session) {
        this.db.saveSessionState(session);
      }
      this.sessionPersistTimers.delete(sessionId);
    }, 180);
    this.sessionPersistTimers.set(sessionId, timer);
  }

  private cancelEmbeddedSessionDetection(sessionId: string): void {
    const timer = this.embeddedSessionDetectTimers.get(sessionId);
    if (timer) {
      clearTimeout(timer);
      this.embeddedSessionDetectTimers.delete(sessionId);
    }
  }

  private scheduleEmbeddedSessionDetection(
    session: SessionSnapshot,
    launchedAt: number,
  ): void {
    if (session.embeddedSession || !getEmbeddedSessionProvider(session.kind)) {
      return;
    }

    this.cancelEmbeddedSessionDetection(session.id);

    const runAttempt = async () => {
      const liveSession = this.ptySidecar.getSession(session.id);
      if (!liveSession || liveSession.state !== "live") {
        this.cancelEmbeddedSessionDetection(session.id);
        return;
      }

      if (liveSession.embeddedSession) {
        this.cancelEmbeddedSessionDetection(session.id);
        return;
      }

      const embeddedSession = await detectEmbeddedSession(liveSession.kind, {
        cwd: liveSession.cwd,
        launchedAt,
        correlationId: liveSession.embeddedSessionCorrelationId,
      });
      if (embeddedSession) {
        const resumeLaunch = buildTerminalLaunch(liveSession.kind, embeddedSession);
        liveSession.embeddedSession = embeddedSession;
        liveSession.embeddedSessionCorrelationId = null;
        liveSession.command = resumeLaunch.displayCommand;
        this.db.saveSessionState(liveSession);
        this.syncTerminalPaneSnapshot(liveSession.workspaceId, liveSession.paneId, (payload) => ({
          ...payload,
          command: resumeLaunch.displayCommand,
          embeddedSession,
          embeddedSessionCorrelationId: null,
        }));
        this.broadcast({
          type: "terminal-session-update",
          payload: {
            workspaceId: liveSession.workspaceId,
            paneId: liveSession.paneId,
            sessionId: liveSession.id,
            kind: liveSession.kind,
            cwd: liveSession.cwd,
            command: resumeLaunch.displayCommand,
            sessionState: liveSession.state,
            exitCode: liveSession.exitCode,
            embeddedSession,
            embeddedSessionCorrelationId: null,
          },
        });
        this.cancelEmbeddedSessionDetection(session.id);
        return;
      }

      const timer = setTimeout(() => {
        void runAttempt().catch((error) => {
          console.warn(`[embedded-session] detection failed for ${session.id}: ${String(error)}`);
          this.cancelEmbeddedSessionDetection(session.id);
        });
      }, 2_000);
      this.embeddedSessionDetectTimers.set(session.id, timer);
    };

    const timer = setTimeout(() => {
      void runAttempt().catch((error) => {
        console.warn(`[embedded-session] detection failed for ${session.id}: ${String(error)}`);
        this.cancelEmbeddedSessionDetection(session.id);
      });
    }, 1_500);
    this.embeddedSessionDetectTimers.set(session.id, timer);
  }

  private syncTerminalPaneSnapshot(
    workspaceId: string,
    paneId: string,
    updater: (payload: TerminalPanePayload) => TerminalPanePayload,
  ): void {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      return;
    }

    const snapshot = this.db.getSnapshot(workspaceId);
    if (!snapshot) {
      return;
    }

    const sanitized = sanitizeSnapshot(snapshot, workspace.workspacePath);
    if (!(paneId in sanitized.panes)) {
      return;
    }

    const next = updatePane(sanitized, paneId, (pane) => {
      if (pane.type !== "shell" && pane.type !== "agent-shell") {
        return pane;
      }

      return {
        ...pane,
        payload: updater(pane.payload as TerminalPanePayload),
      };
    });
    this.db.saveSnapshotDocument(next);
  }

  private async refreshWorkspace(workspaceId: string): Promise<void> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      return;
    }

    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      this.db.updateWorkspaceStatus(workspaceId, {
        dirty: false,
        bookmarks: [],
        diffText: "",
        unreadNotes: 0,
        activeAgentCount: 0,
        recentActivityAt: now(),
      });
      const nextWorkspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
      if (nextWorkspace) {
        this.broadcast({
          type: "workspace-status",
          payload: {
            workspace: nextWorkspace,
            notes: [] as NoteRecord[],
          },
        });
      }
      return;
    }

    const [status, notes] = await Promise.all([
      readWorkspaceStatus(workspace.workspacePath),
      this.readNotes(workspace),
    ]);

    const activeAgentCount = new Set(
      this.db
        .listSessionStates(workspaceId)
        .filter((session) => isAgentTerminalKind(session.kind) && session.state !== "stopped")
        .map((session) => session.id),
    ).size;

    this.db.updateWorkspaceStatus(workspaceId, {
      dirty: status.dirty,
      bookmarks: status.bookmarks,
      diffText: status.diffText,
      unreadNotes: notes.filter((note) => note.unread).length,
      activeAgentCount,
      recentActivityAt: now(),
    });

    const nextWorkspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!nextWorkspace) {
      return;
    }

    this.broadcast({
      type: "workspace-status",
      payload: {
        workspace: nextWorkspace,
        notes,
      },
    });
  }

  private updateActiveAgentCounts(workspaceId: string): void {
    const activeAgentCount = new Set(
      this.db
        .listSessionStates(workspaceId)
        .filter((session) => isAgentTerminalKind(session.kind) && session.state !== "stopped")
        .map((session) => session.id),
    ).size;
    this.db.updateWorkspaceStatus(workspaceId, {
      activeAgentCount,
      recentActivityAt: now(),
    });
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (workspace) {
      this.broadcast({
        type: "workspace-status",
        payload: {
          workspace,
          notes: null,
        },
      });
    }
  }

  private async closeWorkspaceRuntime(workspaceId: string): Promise<void> {
    const runtime = this.runtimes.get(workspaceId);
    if (!runtime) {
      return;
    }

    for (const sessionId of runtime.sessionIds) {
      this.cancelEmbeddedSessionDetection(sessionId);
      this.ptySidecar.kill(sessionId);
    }
    this.runtimes.delete(workspaceId);
  }

  async openWorkspace(workspaceId: string, viewportWidth?: number): Promise<WorkspaceDetail> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }

    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      throw new Error(
        `JJ reports no recorded path for workspace "${workspace.workspaceName}". Open it from a real workspace directory or forget that workspace entry in JJ.`,
      );
    }

    this.runtimes.set(workspaceId, {
      workspaceId,
      lastOpenedAt: now(),
      sessionIds: this.runtimes.get(workspaceId)?.sessionIds ?? new Set<string>(),
    });

    let snapshot = this.db.getSnapshot(workspaceId);
    if (!snapshot) {
      snapshot = createDefaultSnapshot(workspaceId, workspace.workspacePath, viewportWidth);
      this.db.saveSnapshot(snapshot);
    }

    const sanitized = sanitizeSnapshot(snapshot, workspace.workspacePath);
    for (const pane of Object.values(sanitized.panes)) {
      if (pane.type !== "shell" && pane.type !== "agent-shell") {
        continue;
      }

      const payload = pane.payload as TerminalPanePayload;
      const savedSession = this.db.getSessionStateByPane(pane.id);
      const liveSessionId = payload.sessionId || savedSession?.id || "";
      const session = liveSessionId ? this.ptySidecar.getSession(liveSessionId) : null;
      Object.assign(
        payload,
        restoreTerminalPanePayload(payload, session ?? null, savedSession),
      );
    }

    const notes = await this.readNotes(workspace);

    this.db.updateWorkspaceOpened(workspaceId);
    void this.refreshWorkspace(workspaceId).catch((error) => {
      const message = error instanceof Error ? error.message : String(error);
      console.error(`[workspace-open] refresh failed for ${workspaceId}: ${message}`);
    });

    return {
      workspace: this.db.listWorkspaces().find((item) => item.id === workspaceId)!,
      snapshot: sanitized,
      notes,
    };
  }

  async saveSnapshot(workspaceId: string, snapshot: WorkspaceSnapshot): Promise<WorkspaceSnapshot> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }

    const existingSessions = this.db.listSessionStates(workspaceId);
    const sanitized = sanitizeSnapshot(snapshot, workspace.workspacePath);
    const nextPaneIds = new Set(
      Object.values(sanitized.panes)
        .filter((pane) => pane.type === "shell" || pane.type === "agent-shell")
        .map((pane) => pane.id),
    );
    for (const session of existingSessions) {
      if (!nextPaneIds.has(session.paneId)) {
        this.ptySidecar.kill(session.id);
      }
    }
    this.db.saveSnapshot(sanitized);
    return sanitized;
  }

  async createWorkspace(payload: CreateWorkspacePayload): Promise<void> {
    const root = this.db.listProjectRoots().find((item) => item.id === payload.rootId);
    if (!root) {
      throw new Error("Project root not found");
    }

    await jjCreateWorkspace(root.rootPath, payload.destinationPath, payload.workspaceName);
    await this.syncProjectRoot(root.id, root.rootPath);
    this.broadcast({
      type: "nav-updated",
      payload: this.getBootstrap(),
    });
  }

  async forgetWorkspace(workspaceId: string): Promise<void> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }

    await jjForgetWorkspace(workspace.rootPath, workspace.workspaceName);
    await this.closeWorkspaceRuntime(workspace.id);
    for (const session of this.db.listSessionStates(workspace.id)) {
      this.ptySidecar.kill(session.id);
    }
    const watcher = this.watchers.get(workspace.id);
    if (watcher) {
      await watcher.close();
      this.watchers.delete(workspace.id);
    }
    this.db.deleteWorkspace(workspace.id);
    this.broadcast({
      type: "nav-updated",
      payload: this.getBootstrap(),
    });
  }

  async createTerminalSession(request: TerminalCreateRequest): Promise<SessionSnapshot> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === request.workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }
    const kind = normalizeTerminalKind(request.kind);
    const savedSession = this.db.getSessionStateByPane(request.paneId);
    const stableSessionId = savedSession?.id || globalThis.crypto.randomUUID();
    const launchedAt = now();

    const runtime =
      this.runtimes.get(workspace.id) ??
      {
        workspaceId: workspace.id,
        lastOpenedAt: now(),
        sessionIds: new Set<string>(),
      };
    this.runtimes.set(workspace.id, runtime);

    const liveSessions = this.ptySidecar
      .listSessions()
      .filter(
        (session) =>
          session.id === stableSessionId &&
          session.state === "live",
      );
    const reusedSession = liveSessions.at(-1) ?? null;
    if (reusedSession) {
      this.logTerminal("reuse-session", {
        workspaceId: workspace.id,
        paneId: request.paneId,
        sessionId: reusedSession.id,
        duplicates: liveSessions.length - 1,
      });
      for (const duplicate of liveSessions.slice(0, -1)) {
        runtime.sessionIds.delete(duplicate.id);
        this.ptySidecar.kill(duplicate.id);
      }
      runtime.sessionIds.add(reusedSession.id);
      this.db.saveSessionState(reusedSession);
      this.syncTerminalPaneSnapshot(workspace.id, request.paneId, (payload) => ({
        ...payload,
        sessionId: reusedSession.id,
        sessionState: reusedSession.state,
        autoStart: false,
        exitCode: reusedSession.exitCode,
        cwd: reusedSession.cwd,
        command: reusedSession.command,
        kind: reusedSession.kind,
        restoredBuffer: "",
        embeddedSession: reusedSession.embeddedSession,
        embeddedSessionCorrelationId: reusedSession.embeddedSessionCorrelationId,
      }));
      this.updateActiveAgentCounts(workspace.id);
      this.scheduleEmbeddedSessionDetection(reusedSession, launchedAt);
      return reusedSession;
    }

    const correlationId =
      savedSession?.embeddedSession
        ? null
        : savedSession?.embeddedSessionCorrelationId ??
          createEmbeddedSessionCorrelationId(launchedAt, stableSessionId);
    const embeddedSession =
      savedSession?.embeddedSession ??
      (await detectEmbeddedSession(kind, {
        cwd: workspace.workspacePath,
        correlationId,
      }));
    const nextCorrelationId = embeddedSession ? null : correlationId;
    const launch = buildTerminalLaunch(kind, embeddedSession, nextCorrelationId);

    const session = this.ptySidecar.createSession({
      sessionId: stableSessionId,
      workspaceId: workspace.id,
      paneId: request.paneId,
      kind,
      cwd: workspace.workspacePath,
      cols: request.cols,
      rows: request.rows,
      launchArgv: launch.argv,
      displayCommand: launch.displayCommand,
      embeddedSession,
      embeddedSessionCorrelationId: nextCorrelationId,
    });
    this.logTerminal("create-session", {
      workspaceId: workspace.id,
      paneId: request.paneId,
      sessionId: session.id,
      kind,
      cols: request.cols,
      rows: request.rows,
    });
    runtime.sessionIds.add(session.id);
    this.db.saveSessionState(session);
    this.syncTerminalPaneSnapshot(workspace.id, request.paneId, (payload) => ({
      ...payload,
      sessionId: session.id,
      sessionState: session.state,
      autoStart: false,
      exitCode: session.exitCode,
      cwd: session.cwd,
      command: session.command,
      kind: session.kind,
      restoredBuffer: "",
      embeddedSession: session.embeddedSession,
      embeddedSessionCorrelationId: session.embeddedSessionCorrelationId,
    }));
    this.updateActiveAgentCounts(workspace.id);
    this.scheduleEmbeddedSessionDetection(session, launchedAt);
    return session;
  }

  getSession(sessionId: string): SessionSnapshot | null {
    const session = this.ptySidecar.getSession(sessionId);
    if (!session) {
      return null;
    }
    return {
      ...session,
      screen: this.ptySidecar.captureScreen(sessionId),
    };
  }

  writeToSession(sessionId: string, data: string): void {
    this.logTerminal("input", () => ({
      sessionId,
      chunk: formatTerminalChunk(data),
    }));
    this.ptySidecar.write(sessionId, data);
  }

  resizeSession(sessionId: string, cols: number, rows: number): void {
    this.logTerminal("resize", {
      sessionId,
      cols,
      rows,
    });
    this.ptySidecar.resize(sessionId, cols, rows);
  }

  detachSession(sessionId: string): void {
    this.logTerminal("detach", {
      sessionId,
    });
    for (const runtime of this.runtimes.values()) {
      runtime.sessionIds.delete(sessionId);
    }
    this.cancelEmbeddedSessionDetection(sessionId);
    this.ptySidecar.detach(sessionId);
  }

  closeSession(sessionId: string): void {
    this.logTerminal("close", {
      sessionId,
    });
    for (const runtime of this.runtimes.values()) {
      runtime.sessionIds.delete(sessionId);
    }
    this.cancelEmbeddedSessionDetection(sessionId);
    this.ptySidecar.kill(sessionId);
  }

  async createNote(workspaceId: string, fileName: string): Promise<NoteRecord> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }
    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      throw new Error("Workspace has no recorded path");
    }

    const safeName = `${slugifyNote(fileName)}.note.md`;
    const path = join(workspace.workspacePath, safeName);
    const body = `# ${safeName.replace(/\.note\.md$/i, "").replace(/-/g, " ")}\n\n`;
    await writeFile(path, body, "utf8");
    const notes = await this.readNotes(workspace);
    const created = notes.find((note) => note.path === path);
    if (!created) {
      throw new Error("Failed to create note");
    }
    await this.refreshWorkspace(workspaceId);
    return created;
  }

  async saveNote(workspaceId: string, notePath: string, body: string): Promise<NoteRecord> {
    const workspace = this.db.listWorkspaces().find((item) => item.id === workspaceId);
    if (!workspace) {
      throw new Error("Workspace not found");
    }
    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      throw new Error("Workspace has no recorded path");
    }

    await writeFile(notePath, body, "utf8");
    const info = await stat(notePath);
    this.db.upsertNoteState({
      workspaceId,
      path: notePath,
      title: extractNoteTitle(basename(notePath), body),
      lastReadAt: now(),
      lastKnownMtime: Math.floor(info.mtimeMs),
    });
    const notes = await this.readNotes(workspace);
    const note = notes.find((item) => item.path === notePath);
    if (!note) {
      throw new Error("Note not found after save");
    }
    await this.refreshWorkspace(workspaceId);
    return note;
  }

  async markNoteRead(workspaceId: string, notePath: string): Promise<void> {
    const body = await readFile(notePath, "utf8");
    const info = await stat(notePath);
    this.db.upsertNoteState({
      workspaceId,
      path: notePath,
      title: extractNoteTitle(basename(notePath), body),
      lastReadAt: now(),
      lastKnownMtime: Math.floor(info.mtimeMs),
    });
    await this.refreshWorkspace(workspaceId);
  }

  private async readNotes(workspace: WorkspaceSummary): Promise<NoteRecord[]> {
    if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
      return [];
    }

    const entries = await readdir(workspace.workspacePath, { withFileTypes: true });
    const noteState = this.db.listNoteState(workspace.id);
    const notes: NoteRecord[] = [];

    for (const entry of entries) {
      if (!entry.isFile() || !entry.name.endsWith(".note.md")) {
        continue;
      }

      const absolutePath = join(workspace.workspacePath, entry.name);
      const [body, info] = await Promise.all([
        readFile(absolutePath, "utf8"),
        stat(absolutePath),
      ]);
      const mtime = Math.floor(info.mtimeMs);
      const persisted = noteState[absolutePath];
      const lastReadAt = persisted?.lastReadAt ?? 0;
      const title = extractNoteTitle(entry.name, body);

      this.db.upsertNoteState({
        workspaceId: workspace.id,
        path: absolutePath,
        title,
        lastReadAt,
        lastKnownMtime: mtime,
      });

      notes.push({
        workspaceId: workspace.id,
        path: absolutePath,
        fileName: entry.name,
        title,
        body,
        unread: mtime > lastReadAt,
        updatedAt: mtime,
        lastReadAt,
      });
    }

    this.db.deleteMissingNotes(workspace.id, notes.map((note) => note.path));
    return notes.sort((left, right) => right.updatedAt - left.updatedAt);
  }
}

import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { Database } from "bun:sqlite";
import type {
  BrowserRefRecord,
  ProjectRootRecord,
  SessionSnapshot,
  WorkspaceSnapshot,
  WorkspaceSummary,
} from "../shared/types";
import { normalizeTerminalKind } from "../shared/terminal-kind";

export class AppDatabase {
  private readonly db: Database;

  constructor(dbPath: string) {
    mkdirSync(dirname(dbPath), { recursive: true });
    this.db = new Database(dbPath);
    this.db.exec("pragma journal_mode = WAL;");
    this.db.exec("pragma foreign_keys = ON;");
    this.init();
  }

  private init(): void {
    this.db.exec(`
      create table if not exists project_roots (
        id text primary key,
        root_path text not null unique,
        label text not null,
        created_at integer not null,
        updated_at integer not null
      );

      create table if not exists workspaces (
        id text primary key,
        root_id text not null references project_roots(id) on delete cascade,
        root_path text not null,
        project_label text not null,
        workspace_name text not null,
        workspace_path text not null unique,
        dirty integer not null default 0,
        bookmarks text not null default '',
        unread_notes integer not null default 0,
        active_agent_count integer not null default 0,
        agent_attention_state text,
        recent_activity_at integer not null default 0,
        diff_text text not null default '',
        created_at integer not null,
        updated_at integer not null,
        last_opened_at integer not null default 0
      );

      create table if not exists workspace_snapshots (
        workspace_id text primary key references workspaces(id) on delete cascade,
        snapshot_json text not null,
        updated_at integer not null
      );

      create table if not exists note_state (
        workspace_id text not null references workspaces(id) on delete cascade,
        note_path text not null,
        title text not null default '',
        last_read_at integer not null default 0,
        last_known_mtime integer not null default 0,
        primary key (workspace_id, note_path)
      );

      create table if not exists browser_refs (
        workspace_id text not null references workspaces(id) on delete cascade,
        pane_id text not null,
        url text not null,
        title text not null default '',
        updated_at integer not null,
        primary key (workspace_id, pane_id)
      );

      create table if not exists session_state (
        pane_id text primary key,
        workspace_id text not null references workspaces(id) on delete cascade,
        session_id text,
        kind text not null,
        cwd text not null,
        command text not null,
        state text not null,
        exit_code integer,
        embedded_session_json text,
        embedded_session_correlation_id text,
        agent_attention_state text,
        buffer text not null default '',
        updated_at integer not null
      );
    `);

    const sessionStateColumns = this.db
      .prepare("pragma table_info(session_state)")
      .all() as Array<{ name: string }>;
    if (!sessionStateColumns.some((column) => column.name === "buffer")) {
      this.db.exec("alter table session_state add column buffer text not null default ''");
    }
    if (!sessionStateColumns.some((column) => column.name === "embedded_session_json")) {
      this.db.exec("alter table session_state add column embedded_session_json text");
    }
    if (!sessionStateColumns.some((column) => column.name === "embedded_session_correlation_id")) {
      this.db.exec("alter table session_state add column embedded_session_correlation_id text");
    }
    if (!sessionStateColumns.some((column) => column.name === "agent_attention_state")) {
      this.db.exec("alter table session_state add column agent_attention_state text");
    }
    const workspaceColumns = this.db
      .prepare("pragma table_info(workspaces)")
      .all() as Array<{ name: string }>;
    if (!workspaceColumns.some((column) => column.name === "agent_attention_state")) {
      this.db.exec("alter table workspaces add column agent_attention_state text");
    }
  }

  close(): void {
    this.db.close();
  }

  upsertProjectRoot(root: ProjectRootRecord): void {
    this.db
      .prepare(`
        insert into project_roots (id, root_path, label, created_at, updated_at)
        values (?, ?, ?, ?, ?)
        on conflict(id) do update set
          root_path = excluded.root_path,
          label = excluded.label,
          updated_at = excluded.updated_at
      `)
      .run(root.id, root.rootPath, root.label, root.createdAt, root.updatedAt);
  }

  listProjectRoots(): ProjectRootRecord[] {
    return this.db
      .prepare(`
        select
          id,
          root_path as rootPath,
          label,
          created_at as createdAt,
          updated_at as updatedAt
        from project_roots
        order by label asc
      `)
      .all() as ProjectRootRecord[];
  }

  deleteProjectRoot(rootId: string): void {
    this.db.prepare("delete from project_roots where id = ?").run(rootId);
  }

  upsertWorkspace(workspace: WorkspaceSummary): void {
    this.db
      .prepare(`
        insert into workspaces (
          id,
          root_id,
          root_path,
          project_label,
          workspace_name,
          workspace_path,
          dirty,
          bookmarks,
          unread_notes,
          active_agent_count,
          agent_attention_state,
          recent_activity_at,
          diff_text,
          created_at,
          updated_at,
          last_opened_at
        ) values (
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?,
          ?
        )
        on conflict(id) do update set
          root_id = excluded.root_id,
          root_path = excluded.root_path,
          project_label = excluded.project_label,
          workspace_name = excluded.workspace_name,
          workspace_path = excluded.workspace_path,
          dirty = excluded.dirty,
          bookmarks = excluded.bookmarks,
          unread_notes = excluded.unread_notes,
          active_agent_count = excluded.active_agent_count,
          agent_attention_state = excluded.agent_attention_state,
          recent_activity_at = excluded.recent_activity_at,
          diff_text = excluded.diff_text,
          updated_at = excluded.updated_at,
          last_opened_at = excluded.last_opened_at
      `)
      .run(
        workspace.id,
        workspace.rootId,
        workspace.rootPath,
        workspace.projectLabel,
        workspace.workspaceName,
        workspace.workspacePath,
        workspace.dirty ? 1 : 0,
        JSON.stringify(workspace.bookmarks),
        workspace.unreadNotes,
        workspace.activeAgentCount,
        workspace.agentAttentionState,
        workspace.recentActivityAt,
        workspace.diffText,
        workspace.createdAt,
        workspace.updatedAt,
        workspace.lastOpenedAt,
      );
  }

  listWorkspaces(): WorkspaceSummary[] {
    const rows = this.db
      .prepare(`
        select
          id,
          root_id as rootId,
          root_path as rootPath,
          project_label as projectLabel,
          workspace_name as workspaceName,
          workspace_path as workspacePath,
          dirty,
          bookmarks,
          unread_notes as unreadNotes,
          active_agent_count as activeAgentCount,
          agent_attention_state as agentAttentionState,
          recent_activity_at as recentActivityAt,
          diff_text as diffText,
          created_at as createdAt,
          updated_at as updatedAt,
          last_opened_at as lastOpenedAt
        from workspaces
        order by project_label asc, workspace_name asc
      `)
      .all() as Array<Record<string, unknown>>;

    return rows.map((row) => ({
      id: String(row.id),
      rootId: String(row.rootId),
      rootPath: String(row.rootPath),
      projectLabel: String(row.projectLabel),
      workspaceName: String(row.workspaceName),
      workspacePath: String(row.workspacePath),
      dirty: Number(row.dirty) === 1,
      bookmarks: JSON.parse(String(row.bookmarks)),
      unreadNotes: Number(row.unreadNotes),
      activeAgentCount: Number(row.activeAgentCount),
      agentAttentionState:
        typeof row.agentAttentionState === "string" ? String(row.agentAttentionState) as WorkspaceSummary["agentAttentionState"] : null,
      recentActivityAt: Number(row.recentActivityAt),
      diffText: String(row.diffText),
      createdAt: Number(row.createdAt),
      updatedAt: Number(row.updatedAt),
      lastOpenedAt: Number(row.lastOpenedAt),
    }));
  }

  deleteWorkspace(workspaceId: string): void {
    this.db.prepare("delete from workspaces where id = ?").run(workspaceId);
  }

  deleteWorkspacesMissing(rootId: string, workspaceIds: string[]): void {
    const rows = this.db
      .prepare("select id from workspaces where root_id = ?")
      .all(rootId) as Array<{ id: string }>;
    for (const row of rows) {
      if (!workspaceIds.includes(row.id)) {
        this.deleteWorkspace(row.id);
      }
    }
  }

  updateWorkspaceStatus(workspaceId: string, updates: Partial<WorkspaceSummary>): void {
    const current = this.listWorkspaces().find((workspace) => workspace.id === workspaceId);
    if (!current) {
      return;
    }

    this.upsertWorkspace({
      ...current,
      ...updates,
      updatedAt: Date.now(),
    });
  }

  saveSnapshotDocument(snapshot: WorkspaceSnapshot): void {
    this.db
      .prepare(`
        insert into workspace_snapshots (workspace_id, snapshot_json, updated_at)
        values (?, ?, ?)
        on conflict(workspace_id) do update set
          snapshot_json = excluded.snapshot_json,
          updated_at = excluded.updated_at
      `)
      .run(snapshot.workspaceId, JSON.stringify(snapshot), Date.now());
  }

  saveSnapshot(snapshot: WorkspaceSnapshot): void {
    const transaction = this.db.transaction((value: WorkspaceSnapshot) => {
      const existingSessionRows = this.db
        .prepare(`
          select
            pane_id as paneId,
            session_id as sessionId,
            kind,
            cwd,
            command,
            state,
            exit_code as exitCode,
            embedded_session_json as embeddedSessionJson,
            embedded_session_correlation_id as embeddedSessionCorrelationId,
            agent_attention_state as agentAttentionState
          from session_state
          where workspace_id = ?
        `)
        .all(value.workspaceId) as Array<{
          paneId: string;
          sessionId: string | null;
          kind: string;
          cwd: string;
          command: string;
          state: string;
          exitCode: number | null;
          embeddedSessionJson: string | null;
          embeddedSessionCorrelationId: string | null;
          agentAttentionState: SessionSnapshot["agentAttentionState"];
        }>;
      const existingBuffers = Object.fromEntries(
        existingSessionRows.map((row) => [row.paneId, row]),
      );

      this.saveSnapshotDocument(value);

      this.db.prepare("delete from browser_refs where workspace_id = ?").run(value.workspaceId);
      this.db.prepare("delete from session_state where workspace_id = ?").run(value.workspaceId);

      for (const pane of Object.values(value.panes)) {
        if (pane.type === "browser") {
          const payload = pane.payload as { url: string; title: string };
          this.db
            .prepare(`
              insert into browser_refs (workspace_id, pane_id, url, title, updated_at)
              values (?, ?, ?, ?, ?)
            `)
            .run(value.workspaceId, pane.id, payload.url, payload.title, Date.now());
        }

        if (pane.type === "shell" || pane.type === "agent-shell") {
          const payload = pane.payload as {
            sessionId: string | null;
            kind: string;
            cwd: string;
            command: string;
            sessionState: string;
            exitCode: number | null;
            restoredBuffer: string;
            embeddedSession: unknown;
            embeddedSessionCorrelationId: string | null;
            agentAttentionState: SessionSnapshot["agentAttentionState"];
          };
          const existingSession = existingBuffers[pane.id] as
            | {
                sessionId: string | null;
                kind: string;
                cwd: string;
                command: string;
                state: string;
                exitCode: number | null;
                embeddedSessionJson: string | null;
                embeddedSessionCorrelationId: string | null;
                agentAttentionState: SessionSnapshot["agentAttentionState"];
              }
            | undefined;
          const persistedSessionId = payload.sessionId || existingSession?.sessionId || null;
          const persistedKind = normalizeTerminalKind(payload.kind);
          const persistedCommand =
            payload.command || existingSession?.command || "";
          const persistedCwd = payload.cwd || existingSession?.cwd || "";
          const persistedState = payload.sessionState || existingSession?.state || "missing";
          const persistedExitCode =
            payload.exitCode ?? existingSession?.exitCode ?? null;
          const persistedEmbeddedSessionJson =
            payload.embeddedSession != null
              ? JSON.stringify(payload.embeddedSession)
              : existingSession?.embeddedSessionJson ?? null;
          const persistedEmbeddedSessionCorrelationId =
            payload.embeddedSessionCorrelationId ??
            existingSession?.embeddedSessionCorrelationId ??
            null;
          const persistedAgentAttentionState =
            payload.agentAttentionState ??
            existingSession?.agentAttentionState ??
            null;
          this.db
            .prepare(`
              insert into session_state (
                pane_id,
                workspace_id,
                session_id,
                kind,
                cwd,
                command,
                state,
                exit_code,
                embedded_session_json,
                embedded_session_correlation_id,
                agent_attention_state,
                buffer,
                updated_at
              ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            `)
            .run(
              pane.id,
              value.workspaceId,
              persistedSessionId,
              persistedKind,
              persistedCwd,
              persistedCommand,
              persistedState,
              persistedExitCode,
              persistedEmbeddedSessionJson,
              persistedEmbeddedSessionCorrelationId,
              persistedAgentAttentionState,
              "",
              Date.now(),
            );
        }
      }
    });

    transaction(snapshot);
  }

  getSnapshot(workspaceId: string): WorkspaceSnapshot | null {
    const row = this.db
      .prepare(`
        select snapshot_json as snapshotJson
        from workspace_snapshots
        where workspace_id = ?
      `)
      .get(workspaceId) as { snapshotJson: string } | undefined;

    if (!row) {
      return null;
    }

    return JSON.parse(row.snapshotJson) as WorkspaceSnapshot;
  }

  listBrowserRefs(workspaceId: string): BrowserRefRecord[] {
    return this.db
      .prepare(`
        select
          workspace_id as workspaceId,
          pane_id as paneId,
          url,
          title,
          updated_at as updatedAt
        from browser_refs
        where workspace_id = ?
        order by updated_at desc
      `)
      .all(workspaceId) as BrowserRefRecord[];
  }

  upsertNoteState(note: {
    workspaceId: string;
    path: string;
    title: string;
    lastReadAt: number;
    lastKnownMtime: number;
  }): void {
    this.db
      .prepare(`
        insert into note_state (workspace_id, note_path, title, last_read_at, last_known_mtime)
        values (?, ?, ?, ?, ?)
        on conflict(workspace_id, note_path) do update set
          title = excluded.title,
          last_read_at = excluded.last_read_at,
          last_known_mtime = excluded.last_known_mtime
      `)
      .run(
        note.workspaceId,
        note.path,
        note.title,
        note.lastReadAt,
        note.lastKnownMtime,
      );
  }

  deleteMissingNotes(workspaceId: string, notePaths: string[]): void {
    const rows = this.db
      .prepare(`
        select note_path as path
        from note_state
        where workspace_id = ?
      `)
      .all(workspaceId) as Array<{ path: string }>;

    for (const row of rows) {
      if (!notePaths.includes(row.path)) {
        this.db
          .prepare("delete from note_state where workspace_id = ? and note_path = ?")
          .run(workspaceId, row.path);
      }
    }
  }

  listNoteState(workspaceId: string): Record<string, { title: string; lastReadAt: number; lastKnownMtime: number }> {
    const rows = this.db
      .prepare(`
        select
          note_path as path,
          title,
          last_read_at as lastReadAt,
          last_known_mtime as lastKnownMtime
        from note_state
        where workspace_id = ?
      `)
      .all(workspaceId) as Array<{
        path: string;
        title: string;
        lastReadAt: number;
        lastKnownMtime: number;
      }>;

    return Object.fromEntries(rows.map((row) => [row.path, row]));
  }

  saveSessionState(session: SessionSnapshot): void {
    this.db
      .prepare(`
        insert into session_state (
          pane_id,
          workspace_id,
          session_id,
          kind,
          cwd,
          command,
          state,
          exit_code,
          embedded_session_json,
          embedded_session_correlation_id,
          agent_attention_state,
          buffer,
          updated_at
        ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        on conflict(pane_id) do update set
          session_id = excluded.session_id,
          kind = excluded.kind,
          cwd = excluded.cwd,
          command = excluded.command,
          state = excluded.state,
          exit_code = excluded.exit_code,
          embedded_session_json = excluded.embedded_session_json,
          embedded_session_correlation_id = excluded.embedded_session_correlation_id,
          agent_attention_state = excluded.agent_attention_state,
          buffer = excluded.buffer,
          updated_at = excluded.updated_at
      `)
      .run(
        session.paneId,
        session.workspaceId,
        session.id,
        session.kind,
        session.cwd,
        session.command,
        session.state,
        session.exitCode,
        session.embeddedSession ? JSON.stringify(session.embeddedSession) : null,
        session.embeddedSessionCorrelationId,
        session.agentAttentionState,
        session.buffer,
        Date.now(),
      );
  }

  getSessionStateByPane(paneId: string): SessionSnapshot | null {
    const row = this.db
      .prepare(`
        select
          session_id as id,
          workspace_id as workspaceId,
          pane_id as paneId,
          kind,
          cwd,
          command,
          buffer,
          state,
          exit_code as exitCode,
          embedded_session_json as embeddedSessionJson,
          embedded_session_correlation_id as embeddedSessionCorrelationId,
          agent_attention_state as agentAttentionState
        from session_state
        where pane_id = ?
      `)
      .get(paneId) as
      | {
          id: string | null;
          workspaceId: string;
          paneId: string;
          kind: string;
          cwd: string;
          command: string;
          buffer: string;
          state: "live" | "stopped" | "missing";
          exitCode: number | null;
          embeddedSessionJson: string | null;
          embeddedSessionCorrelationId: string | null;
          agentAttentionState: SessionSnapshot["agentAttentionState"];
        }
      | undefined;

    if (!row) {
      return null;
    }

    return {
      id: row.id ?? "",
      workspaceId: row.workspaceId,
      paneId: row.paneId,
      kind: normalizeTerminalKind(row.kind),
      cwd: row.cwd,
      command: row.command,
      buffer: row.buffer,
      state: row.state,
      exitCode: row.exitCode,
      embeddedSession: row.embeddedSessionJson ? JSON.parse(row.embeddedSessionJson) : null,
      embeddedSessionCorrelationId: row.embeddedSessionCorrelationId,
      agentAttentionState: row.agentAttentionState,
    };
  }

  listSessionStates(workspaceId: string): SessionSnapshot[] {
    const rows = this.db
      .prepare(`
        select
          session_id as id,
          workspace_id as workspaceId,
          pane_id as paneId,
          kind,
          cwd,
          command,
          buffer,
          state,
          exit_code as exitCode,
          embedded_session_json as embeddedSessionJson,
          embedded_session_correlation_id as embeddedSessionCorrelationId,
          agent_attention_state as agentAttentionState
        from session_state
        where workspace_id = ?
      `)
      .all(workspaceId) as Array<{
        id: string | null;
        workspaceId: string;
        paneId: string;
        kind: string;
        cwd: string;
        command: string;
        buffer: string;
        state: "live" | "stopped" | "missing";
        exitCode: number | null;
        embeddedSessionJson: string | null;
        embeddedSessionCorrelationId: string | null;
        agentAttentionState: SessionSnapshot["agentAttentionState"];
      }>;

    return rows
      .filter((row) => Boolean(row.id))
      .map((row) => ({
        id: row.id!,
        workspaceId: row.workspaceId,
        paneId: row.paneId,
        kind: normalizeTerminalKind(row.kind),
        cwd: row.cwd,
        command: row.command,
        buffer: row.buffer,
        state: row.state,
        exitCode: row.exitCode,
        embeddedSession: row.embeddedSessionJson ? JSON.parse(row.embeddedSessionJson) : null,
        embeddedSessionCorrelationId: row.embeddedSessionCorrelationId,
        agentAttentionState: row.agentAttentionState,
      }));
  }

  updateWorkspaceOpened(workspaceId: string): void {
    this.db
      .prepare("update workspaces set last_opened_at = ?, updated_at = ? where id = ?")
      .run(Date.now(), Date.now(), workspaceId);
  }
}

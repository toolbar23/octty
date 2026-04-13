export type PaneType = "shell" | "agent-shell" | "note" | "browser" | "diff";
export type TerminalKind = "shell" | "codex" | "pi" | "nvim" | "jjui";
export type PanePlacement = "new-column" | "stack";
export type SidebarTarget = "left" | "right";
export type ColumnPin = SidebarTarget | null;
export type SessionState = "live" | "stopped" | "missing";
export type AgentAttentionState = "idle-seen" | "thinking" | "idle-unseen";
export type WorkspaceBookmarkRelation = "none" | "exact" | "above";
export type WorkspaceState =
  | "published"
  | "merged-local"
  | "draft"
  | "conflicted"
  | "unknown";

export interface EmbeddedSessionRef {
  provider: string;
  id: string;
  label: string | null;
  detectedAt: number;
}

export interface ProjectRootRecord {
  id: string;
  rootPath: string;
  label: string;
  createdAt: number;
  updatedAt: number;
}

export interface WorkspaceStatus {
  workspaceState: WorkspaceState;
  hasWorkingCopyChanges: boolean;
  effectiveAddedLines: number;
  effectiveRemovedLines: number;
  bookmarks: string[];
  bookmarkRelation: WorkspaceBookmarkRelation;
  unreadNotes: number;
  activeAgentCount: number;
  agentAttentionState: AgentAttentionState | null;
  recentActivityAt: number;
  diffText: string;
}

export interface WorkspaceSummary extends WorkspaceStatus {
  id: string;
  rootId: string;
  rootPath: string;
  projectLabel: string;
  workspaceName: string;
  workspacePath: string;
  createdAt: number;
  updatedAt: number;
  lastOpenedAt: number;
}

export interface NoteRecord {
  workspaceId: string;
  path: string;
  fileName: string;
  title: string;
  body: string;
  unread: boolean;
  updatedAt: number;
  lastReadAt: number;
}

export interface TerminalPanePayload {
  kind: TerminalKind;
  sessionId: string | null;
  sessionState: SessionState;
  cwd: string;
  command: string;
  exitCode: number | null;
  autoStart: boolean;
  restoredBuffer: string;
  embeddedSession: EmbeddedSessionRef | null;
  embeddedSessionCorrelationId: string | null;
  agentAttentionState: AgentAttentionState | null;
}

export interface NotePanePayload {
  notePath: string | null;
}

export interface BrowserPanePayload {
  url: string;
  title: string;
}

export interface DiffPanePayload {
  pinned: boolean;
}

export type PanePayload =
  | TerminalPanePayload
  | NotePanePayload
  | BrowserPanePayload
  | DiffPanePayload;

export interface PaneState {
  id: string;
  type: PaneType;
  title: string;
  payload: PanePayload;
}

export interface WorkspaceColumn {
  id: string;
  paneIds: string[];
  widthPx: number;
  heightFractions: number[];
  pinned: ColumnPin;
}

export interface WorkspaceSnapshot {
  layoutVersion: number;
  workspaceId: string;
  activePaneId: string | null;
  panes: Record<string, PaneState>;
  columns: Record<string, WorkspaceColumn>;
  centerColumnIds: string[];
  pinnedLeftColumnId: string | null;
  pinnedRightColumnId: string | null;
  updatedAt: number;
}

export interface WorkspaceDetail {
  workspace: WorkspaceSummary;
  snapshot: WorkspaceSnapshot;
  notes: NoteRecord[];
}

export interface SessionSnapshot {
  id: string;
  workspaceId: string;
  paneId: string;
  kind: TerminalKind;
  cwd: string;
  command: string;
  buffer: string;
  screen?: string;
  state: SessionState;
  exitCode: number | null;
  embeddedSession: EmbeddedSessionRef | null;
  embeddedSessionCorrelationId: string | null;
  agentAttentionState: AgentAttentionState | null;
}

export interface BootstrapPayload {
  projectRoots: ProjectRootRecord[];
  workspaces: WorkspaceSummary[];
}

export interface TerminalCreateRequest {
  workspaceId: string;
  paneId: string;
  kind: TerminalKind;
  cols: number;
  rows: number;
}

export interface WorkspaceSnapshotPayload {
  snapshot: WorkspaceSnapshot;
}

export interface OpenWorkspacePayload {
  viewportWidth?: number;
}

export interface CreateProjectRootPayload {
  path: string;
}

export interface CreateWorkspacePayload {
  rootId: string;
  destinationPath: string;
  workspaceName?: string;
}

export interface CreateNotePayload {
  fileName: string;
}

export interface SaveNotePayload {
  path: string;
  body: string;
}

export interface MarkNoteReadPayload {
  path: string;
}

export interface PickDirectoryPayload {
  startingFolder?: string;
}

export interface BrowserRefRecord {
  workspaceId: string;
  paneId: string;
  url: string;
  title: string;
  updatedAt: number;
}

export interface WorkspaceEventEnvelope {
  type:
    | "nav-updated"
    | "workspace-status"
    | "workspace-detail"
    | "terminal-session-update"
    | "terminal-focus"
    | "terminal-output"
    | "terminal-exit";
  payload: unknown;
}

export const MISSING_WORKSPACE_PATH_PREFIX = "jj-missing://";

export function encodeMissingWorkspacePath(workspaceName: string): string {
  return `${MISSING_WORKSPACE_PATH_PREFIX}${encodeURIComponent(workspaceName)}`;
}

export function hasRecordedWorkspacePath(workspacePath: string): boolean {
  return !workspacePath.startsWith(MISSING_WORKSPACE_PATH_PREFIX);
}

export function displayWorkspacePath(workspacePath: string): string {
  if (!hasRecordedWorkspacePath(workspacePath)) {
    return "(no recorded path)";
  }

  return workspacePath;
}

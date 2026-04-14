import type {
  BootstrapPayload,
  CreateWorkspacePayload,
  NoteRecord,
  ProjectRootRecord,
  SessionSnapshot,
  TerminalCreateRequest,
  WorkspaceDetail,
  WorkspaceEventEnvelope,
  WorkspaceSnapshot,
  WorkspaceSummary,
} from "./types";

export const OCTTY_EVENT_CHANNEL = "octty:event";

export interface OcttyDesktopBridge {
  readonly platform: NodeJS.Platform;
  getBootstrap(): Promise<BootstrapPayload>;
  pickDirectory(startingFolder?: string): Promise<string | null>;
  addProjectRoot(path: string): Promise<ProjectRootRecord>;
  removeProjectRoot(rootId: string): Promise<void>;
  updateProjectRootDisplayName(rootId: string, displayName: string): Promise<ProjectRootRecord>;
  createWorkspace(payload: CreateWorkspacePayload): Promise<WorkspaceSummary>;
  updateWorkspaceDisplayName(workspaceId: string, displayName: string): Promise<WorkspaceSummary>;
  forgetWorkspace(workspaceId: string): Promise<void>;
  deleteAndForgetWorkspace(workspaceId: string): Promise<void>;
  openWorkspace(workspaceId: string, viewportWidth?: number): Promise<WorkspaceDetail>;
  saveSnapshot(workspaceId: string, snapshot: WorkspaceSnapshot): Promise<WorkspaceSnapshot>;
  createNote(workspaceId: string, fileName: string): Promise<NoteRecord>;
  saveNote(workspaceId: string, notePath: string, body: string): Promise<NoteRecord>;
  markNoteRead(workspaceId: string, notePath: string): Promise<void>;
  createTerminalSession(request: TerminalCreateRequest): Promise<SessionSnapshot>;
  getSession(sessionId: string): Promise<SessionSnapshot>;
  sendTerminalInput(sessionId: string, data: string): void;
  resizeTerminal(sessionId: string, cols: number, rows: number): void;
  focusTerminal(sessionId: string, focused: boolean): void;
  detachTerminal(sessionId: string): void;
  closeTerminal(sessionId: string): void;
  openExternal(url: string): Promise<void>;
  onWorkspaceEvent(listener: (event: WorkspaceEventEnvelope) => void): () => void;
}

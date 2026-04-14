import type {
  CreateWorkspacePayload,
  NoteRecord,
  ProjectRootRecord,
  SessionSnapshot,
  TerminalCreateRequest,
  WorkspaceDetail,
  WorkspaceEventEnvelope,
  WorkspaceSnapshot,
  WorkspaceSummary,
} from "../shared/types";
import { WorkspaceService } from "../backend/service";

type EventListener = (event: WorkspaceEventEnvelope) => void;

export class OcttyBackend {
  private readonly service = new WorkspaceService();
  private readonly listeners = new Set<EventListener>();
  private removeClient: (() => void) | null = null;

  async init(): Promise<void> {
    await this.service.init();
    this.removeClient = this.service.addClient((message) => {
      this.emit(message as WorkspaceEventEnvelope);
    });
  }

  dispose(): void {
    this.removeClient?.();
    this.removeClient = null;
    this.listeners.clear();
    this.service.dispose();
  }

  onEvent(listener: EventListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private emit(event: WorkspaceEventEnvelope): void {
    for (const listener of this.listeners) {
      listener(event);
    }
  }

  getBootstrap() {
    return this.service.getBootstrap();
  }

  addProjectRoot(path: string): Promise<ProjectRootRecord> {
    return this.service.addProjectRoot(path);
  }

  removeProjectRoot(rootId: string): Promise<void> {
    return this.service.removeProjectRoot(rootId);
  }

  updateProjectRootDisplayName(rootId: string, displayName: string): Promise<ProjectRootRecord> {
    return this.service.updateProjectRootDisplayName(rootId, displayName);
  }

  createWorkspace(payload: CreateWorkspacePayload): Promise<WorkspaceSummary> {
    return this.service.createWorkspace(payload);
  }

  updateWorkspaceDisplayName(
    workspaceId: string,
    displayName: string,
  ): Promise<WorkspaceSummary> {
    return this.service.updateWorkspaceDisplayName(workspaceId, displayName);
  }

  forgetWorkspace(workspaceId: string): Promise<void> {
    return this.service.forgetWorkspace(workspaceId);
  }

  deleteAndForgetWorkspace(workspaceId: string): Promise<void> {
    return this.service.deleteAndForgetWorkspace(workspaceId);
  }

  openWorkspace(workspaceId: string, viewportWidth?: number): Promise<WorkspaceDetail> {
    return this.service.openWorkspace(workspaceId, viewportWidth);
  }

  saveSnapshot(workspaceId: string, snapshot: WorkspaceSnapshot): Promise<WorkspaceSnapshot> {
    return this.service.saveSnapshot(workspaceId, snapshot);
  }

  createNote(workspaceId: string, fileName: string): Promise<NoteRecord> {
    return this.service.createNote(workspaceId, fileName);
  }

  saveNote(workspaceId: string, notePath: string, body: string): Promise<NoteRecord> {
    return this.service.saveNote(workspaceId, notePath, body);
  }

  markNoteRead(workspaceId: string, notePath: string): Promise<void> {
    return this.service.markNoteRead(workspaceId, notePath);
  }

  createTerminalSession(request: TerminalCreateRequest): Promise<SessionSnapshot> {
    return this.service.createTerminalSession(request);
  }

  getSession(sessionId: string): SessionSnapshot | null {
    return this.service.getSession(sessionId);
  }

  sendTerminalInput(sessionId: string, data: string): void {
    this.service.writeToSession(sessionId, data);
  }

  resizeTerminal(sessionId: string, cols: number, rows: number): void {
    this.service.resizeSession(sessionId, cols, rows);
  }

  focusTerminal(sessionId: string, focused: boolean): void {
    this.service.setSessionFocused(sessionId, focused);
  }

  detachTerminal(sessionId: string): void {
    this.service.detachSession(sessionId);
  }

  closeTerminal(sessionId: string): void {
    this.service.closeSession(sessionId);
  }
}

import type {
  BootstrapPayload,
  BrowserEventEnvelope,
  BrowserFindResult,
  BrowserViewBounds,
  BrowserViewState,
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
import type { TerminalClipboardPaste } from "./terminal-clipboard";

export const OCTTY_EVENT_CHANNEL = "octty:event";
export const OCTTY_BROWSER_EVENT_CHANNEL = "octty:browser-event";

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
  readTerminalClipboardPaste(): Promise<TerminalClipboardPaste>;
  openExternal(url: string): Promise<void>;
  ensureBrowserPane(
    workspaceId: string,
    paneId: string,
    url: string,
    zoomFactor?: number,
    pendingPopupId?: string | null,
  ): Promise<BrowserViewState>;
  setBrowserBounds(paneId: string, bounds: BrowserViewBounds): void;
  hideBrowserPane(paneId: string): void;
  destroyBrowserPane(paneId: string): void;
  focusBrowserPane(paneId: string): Promise<void>;
  navigateBrowserPane(paneId: string, url: string): Promise<BrowserViewState>;
  goBackBrowserPane(paneId: string): Promise<BrowserViewState>;
  goForwardBrowserPane(paneId: string): Promise<BrowserViewState>;
  reloadBrowserPane(paneId: string): Promise<BrowserViewState>;
  stopBrowserPane(paneId: string): Promise<BrowserViewState>;
  setBrowserZoom(paneId: string, zoomFactor: number): Promise<BrowserViewState>;
  findInBrowserPane(
    paneId: string,
    text: string,
    options?: { forward?: boolean; findNext?: boolean },
  ): Promise<BrowserFindResult | null>;
  stopFindInBrowserPane(
    paneId: string,
    action?: "clearSelection" | "keepSelection" | "activateSelection",
  ): Promise<void>;
  openBrowserDevTools(paneId: string): Promise<void>;
  onBrowserEvent(listener: (event: BrowserEventEnvelope) => void): () => void;
  onWorkspaceEvent(listener: (event: WorkspaceEventEnvelope) => void): () => void;
}

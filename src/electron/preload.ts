import { contextBridge, ipcRenderer } from "electron";
import {
  OCTTY_BROWSER_EVENT_CHANNEL,
  OCTTY_EVENT_CHANNEL,
  type OcttyDesktopBridge,
} from "../shared/desktop-bridge";
import type {
  BrowserEventEnvelope,
  BrowserFindResult,
  BrowserViewBounds,
  BrowserViewState,
  CreateWorkspacePayload,
  SessionSnapshot,
  TerminalCreateRequest,
  WorkspaceEventEnvelope,
  WorkspaceSnapshot,
} from "../shared/types";

const bridge: OcttyDesktopBridge = {
  platform: process.platform,
  getBootstrap: () => ipcRenderer.invoke("octty:get-bootstrap"),
  pickDirectory: (startingFolder?: string) =>
    ipcRenderer.invoke("octty:pick-directory", startingFolder),
  addProjectRoot: (path: string) => ipcRenderer.invoke("octty:add-project-root", path),
  removeProjectRoot: (rootId: string) => ipcRenderer.invoke("octty:remove-project-root", rootId),
  updateProjectRootDisplayName: (rootId: string, displayName: string) =>
    ipcRenderer.invoke("octty:update-project-root-display-name", rootId, displayName),
  createWorkspace: (payload: CreateWorkspacePayload) =>
    ipcRenderer.invoke("octty:create-workspace", payload),
  updateWorkspaceDisplayName: (workspaceId: string, displayName: string) =>
    ipcRenderer.invoke("octty:update-workspace-display-name", workspaceId, displayName),
  forgetWorkspace: (workspaceId: string) =>
    ipcRenderer.invoke("octty:forget-workspace", workspaceId),
  deleteAndForgetWorkspace: (workspaceId: string) =>
    ipcRenderer.invoke("octty:delete-and-forget-workspace", workspaceId),
  openWorkspace: (workspaceId: string, viewportWidth?: number) =>
    ipcRenderer.invoke("octty:open-workspace", workspaceId, viewportWidth),
  saveSnapshot: (workspaceId: string, snapshot: WorkspaceSnapshot) =>
    ipcRenderer.invoke("octty:save-snapshot", workspaceId, snapshot),
  createNote: (workspaceId: string, fileName: string) =>
    ipcRenderer.invoke("octty:create-note", workspaceId, fileName),
  saveNote: (workspaceId: string, notePath: string, body: string) =>
    ipcRenderer.invoke("octty:save-note", workspaceId, notePath, body),
  markNoteRead: (workspaceId: string, notePath: string) =>
    ipcRenderer.invoke("octty:mark-note-read", workspaceId, notePath),
  createTerminalSession: (request: TerminalCreateRequest) =>
    ipcRenderer.invoke("octty:create-terminal-session", request),
  getSession: (sessionId: string): Promise<SessionSnapshot> =>
    ipcRenderer.invoke("octty:get-session", sessionId),
  sendTerminalInput: (sessionId: string, data: string) =>
    ipcRenderer.send("octty:terminal-input", { sessionId, data }),
  resizeTerminal: (sessionId: string, cols: number, rows: number) =>
    ipcRenderer.send("octty:terminal-resize", { sessionId, cols, rows }),
  focusTerminal: (sessionId: string, focused: boolean) =>
    ipcRenderer.send("octty:terminal-focus", { sessionId, focused }),
  detachTerminal: (sessionId: string) =>
    ipcRenderer.send("octty:terminal-detach", { sessionId }),
  closeTerminal: (sessionId: string) =>
    ipcRenderer.send("octty:terminal-close", { sessionId }),
  readTerminalClipboardPaste: () =>
    ipcRenderer.invoke("octty:read-terminal-clipboard-paste"),
  openExternal: (url: string) => ipcRenderer.invoke("octty:open-external", url),
  ensureBrowserPane: (
    workspaceId: string,
    paneId: string,
    url: string,
    zoomFactor?: number,
    pendingPopupId?: string | null,
  ): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-ensure", {
      workspaceId,
      paneId,
      url,
      zoomFactor,
      pendingPopupId,
    }),
  setBrowserBounds: (paneId: string, bounds: BrowserViewBounds) =>
    ipcRenderer.send("octty:browser-bounds", { paneId, bounds }),
  hideBrowserPane: (paneId: string) =>
    ipcRenderer.send("octty:browser-hide", { paneId }),
  destroyBrowserPane: (paneId: string) =>
    ipcRenderer.send("octty:browser-destroy", { paneId }),
  focusBrowserPane: (paneId: string): Promise<void> =>
    ipcRenderer.invoke("octty:browser-focus", { paneId }),
  navigateBrowserPane: (paneId: string, url: string): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-navigate", { paneId, url }),
  goBackBrowserPane: (paneId: string): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-back", { paneId }),
  goForwardBrowserPane: (paneId: string): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-forward", { paneId }),
  reloadBrowserPane: (paneId: string): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-reload", { paneId }),
  stopBrowserPane: (paneId: string): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-stop", { paneId }),
  setBrowserZoom: (paneId: string, zoomFactor: number): Promise<BrowserViewState> =>
    ipcRenderer.invoke("octty:browser-zoom", { paneId, zoomFactor }),
  findInBrowserPane: (
    paneId: string,
    text: string,
    options?: { forward?: boolean; findNext?: boolean },
  ): Promise<BrowserFindResult | null> =>
    ipcRenderer.invoke("octty:browser-find", { paneId, text, options }),
  stopFindInBrowserPane: (
    paneId: string,
    action?: "clearSelection" | "keepSelection" | "activateSelection",
  ): Promise<void> =>
    ipcRenderer.invoke("octty:browser-stop-find", { paneId, action }),
  openBrowserDevTools: (paneId: string): Promise<void> =>
    ipcRenderer.invoke("octty:browser-devtools", { paneId }),
  onBrowserEvent: (listener: (event: BrowserEventEnvelope) => void) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: BrowserEventEnvelope) => {
      listener(payload);
    };
    ipcRenderer.on(OCTTY_BROWSER_EVENT_CHANNEL, wrapped);
    return () => {
      ipcRenderer.removeListener(OCTTY_BROWSER_EVENT_CHANNEL, wrapped);
    };
  },
  onWorkspaceEvent: (listener: (event: WorkspaceEventEnvelope) => void) => {
    const wrapped = (_event: Electron.IpcRendererEvent, payload: WorkspaceEventEnvelope) => {
      listener(payload);
    };
    ipcRenderer.on(OCTTY_EVENT_CHANNEL, wrapped);
    return () => {
      ipcRenderer.removeListener(OCTTY_EVENT_CHANNEL, wrapped);
    };
  },
};

ipcRenderer.on("octty:shortcut", (_event, action: string) => {
  window.dispatchEvent(new CustomEvent("octty-shortcut", { detail: action }));
});

contextBridge.exposeInMainWorld("octtyDesktop", bridge);

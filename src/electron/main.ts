import { app, BrowserWindow, dialog, ipcMain, shell } from "electron";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { OCTTY_EVENT_CHANNEL } from "../shared/desktop-bridge";
import { OcttyBackend } from "./backend";

const currentFile = fileURLToPath(import.meta.url);
const currentDir = dirname(currentFile);
const appRoot = resolve(currentDir, "..", "..");
const rendererHtmlPath = join(currentDir, "index.html");
const preloadPath = join(currentDir, "preload.cjs");
const backend = new OcttyBackend();
const windows = new Set<BrowserWindow>();
const DEBUG_ELECTRON_DIAGNOSTICS = process.env.OCTTY_DEBUG_ELECTRON === "1";

function broadcastEvent(event: unknown): void {
  for (const window of windows) {
    if (!window.isDestroyed()) {
      window.webContents.send(OCTTY_EVENT_CHANNEL, event);
    }
  }
}

function registerIpcHandlers(): void {
  ipcMain.handle("octty:get-bootstrap", () => backend.getBootstrap());
  ipcMain.handle("octty:pick-directory", async (_event, startingFolder?: string) => {
    const result = await dialog.showOpenDialog({
      defaultPath: startingFolder,
      properties: ["openDirectory"],
    });
    if (result.canceled || result.filePaths.length === 0) {
      return null;
    }
    return result.filePaths[0] ?? null;
  });
  ipcMain.handle("octty:add-project-root", (_event, path: string) => backend.addProjectRoot(path));
  ipcMain.handle("octty:remove-project-root", (_event, rootId: string) =>
    backend.removeProjectRoot(rootId),
  );
  ipcMain.handle(
    "octty:update-project-root-display-name",
    (_event, rootId: string, displayName: string) =>
      backend.updateProjectRootDisplayName(rootId, displayName),
  );
  ipcMain.handle("octty:create-workspace", (_event, payload) => backend.createWorkspace(payload));
  ipcMain.handle(
    "octty:update-workspace-display-name",
    (_event, workspaceId: string, displayName: string) =>
      backend.updateWorkspaceDisplayName(workspaceId, displayName),
  );
  ipcMain.handle("octty:forget-workspace", (_event, workspaceId: string) =>
    backend.forgetWorkspace(workspaceId),
  );
  ipcMain.handle("octty:delete-and-forget-workspace", (_event, workspaceId: string) =>
    backend.deleteAndForgetWorkspace(workspaceId),
  );
  ipcMain.handle("octty:open-workspace", (_event, workspaceId: string, viewportWidth?: number) =>
    backend.openWorkspace(workspaceId, viewportWidth),
  );
  ipcMain.handle("octty:save-snapshot", (_event, workspaceId: string, snapshot) =>
    backend.saveSnapshot(workspaceId, snapshot),
  );
  ipcMain.handle("octty:create-note", (_event, workspaceId: string, fileName: string) =>
    backend.createNote(workspaceId, fileName),
  );
  ipcMain.handle("octty:save-note", (_event, workspaceId: string, notePath: string, body: string) =>
    backend.saveNote(workspaceId, notePath, body),
  );
  ipcMain.handle("octty:mark-note-read", (_event, workspaceId: string, notePath: string) =>
    backend.markNoteRead(workspaceId, notePath),
  );
  ipcMain.handle("octty:create-terminal-session", (_event, request) =>
    backend.createTerminalSession(request),
  );
  ipcMain.handle("octty:get-session", (_event, sessionId: string) => {
    const session = backend.getSession(sessionId);
    if (!session) {
      throw new Error("Session not found");
    }
    return session;
  });
  ipcMain.handle("octty:open-external", (_event, url: string) => shell.openExternal(url));

  ipcMain.on("octty:terminal-input", (_event, payload) => {
    backend.sendTerminalInput(payload.sessionId, payload.data);
  });
  ipcMain.on("octty:terminal-resize", (_event, payload) => {
    backend.resizeTerminal(payload.sessionId, payload.cols, payload.rows);
  });
  ipcMain.on("octty:terminal-focus", (_event, payload) => {
    backend.focusTerminal(payload.sessionId, payload.focused);
  });
  ipcMain.on("octty:terminal-detach", (_event, payload) => {
    backend.detachTerminal(payload.sessionId);
  });
  ipcMain.on("octty:terminal-close", (_event, payload) => {
    backend.closeTerminal(payload.sessionId);
  });
}

function createMainWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1600,
    height: 1024,
    x: 80,
    y: 48,
    title: "Octty",
    backgroundColor: "#0c0f13",
    webPreferences: {
      preload: preloadPath,
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  windows.add(window);
  window.on("closed", () => {
    windows.delete(window);
  });

  window.webContents.on("did-fail-load", (_event, errorCode, errorDescription, validatedUrl) => {
    console.error("[electron] did-fail-load", {
      errorCode,
      errorDescription,
      validatedUrl,
    });
  });

  window.webContents.on("render-process-gone", (_event, details) => {
    console.error("[electron] render-process-gone", details);
  });

  if (DEBUG_ELECTRON_DIAGNOSTICS) {
    window.webContents.on("console-message", (event: any) => {
      const { level, message, lineNumber, sourceId } = event;
      console.log("[renderer]", {
        level,
        message,
        line: lineNumber,
        sourceId,
      });
    });
  }

  window.webContents.on("did-finish-load", () => {
    if (!DEBUG_ELECTRON_DIAGNOSTICS) {
      return;
    }

    void window.webContents
      .executeJavaScript(
        `(() => ({
          hasBridge: Boolean(window.octtyDesktop),
          rootChildCount: document.getElementById("root")?.childElementCount ?? -1,
          rootText: document.getElementById("root")?.textContent?.slice(0, 120) ?? ""
        }))()`,
        true,
      )
      .then((payload) => {
        console.log("[electron] renderer-diagnostics", payload);
      })
      .catch((error) => {
        console.error("[electron] renderer-diagnostics failed", error);
      });
  });

  window.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: "deny" };
  });

  window.webContents.on("will-navigate", (event, url) => {
    if (url !== window.webContents.getURL()) {
      event.preventDefault();
      void shell.openExternal(url);
    }
  });

  window.webContents.on("before-input-event", (event, input) => {
    const ctrlOrMeta = input.control || input.meta;
    if (!ctrlOrMeta) {
      return;
    }

    const key = input.key.toLowerCase();
    if (key === "w" && !input.shift) {
      event.preventDefault();
      return;
    }

    if (key === "w" && input.shift) {
      event.preventDefault();
      window.webContents.send("octty:shortcut", "close-pane");
      return;
    }

    if (key === "i" && input.shift) {
      event.preventDefault();
      window.webContents.toggleDevTools();
    }
  });

  void window.loadFile(rendererHtmlPath);
  if (process.env.OCTTY_OPEN_DEVTOOLS === "1") {
    window.webContents.openDevTools({ mode: "detach" });
  }

  return window;
}

async function main(): Promise<void> {
  process.env.OCTTY_SOURCE_ROOT ||= appRoot;
  await app.whenReady();
  await backend.init();
  backend.onEvent((event) => {
    broadcastEvent(event);
  });
  registerIpcHandlers();
  createMainWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createMainWindow();
    }
  });
}

app.on("window-all-closed", () => {
  backend.dispose();
  app.quit();
});

void main().catch((error) => {
  console.error("[electron] failed to start", error);
  backend.dispose();
  app.exit(1);
});

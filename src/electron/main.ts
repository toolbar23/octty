import {
  app,
  BrowserWindow,
  Menu,
  dialog,
  ipcMain,
  shell,
  type MenuItemConstructorOptions,
} from "electron";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { assertRuntimeDependencies } from "../backend/requirements";
import {
  workspaceShortcutAccelerator,
  workspaceShortcutActionForIndex,
  workspaceShortcutTargets,
  type AppShortcutAction,
} from "../shared/app-shortcuts";
import type { WorkspaceSummary } from "../shared/types";
import { OCTTY_EVENT_CHANNEL } from "../shared/desktop-bridge";
import { OcttyBackend } from "./backend";
import { BrowserPaneManager } from "./browser-manager";
import { readTerminalClipboardPaste } from "./terminal-clipboard";

const currentFile = fileURLToPath(import.meta.url);
const currentDir = dirname(currentFile);
const appRoot = resolve(currentDir, "..", "..");
const rendererHtmlPath = join(currentDir, "index.html");
const preloadPath = join(currentDir, "preload.cjs");
let backend: OcttyBackend | null = null;
const browserPaneManager = new BrowserPaneManager();
const windows = new Set<BrowserWindow>();
const DEBUG_ELECTRON_DIAGNOSTICS = process.env.OCTTY_DEBUG_ELECTRON === "1";
const GITHUB_URL = "https://github.com/toolbar23/octty";
const LICENSE_URL = `${GITHUB_URL}/blob/main/LICENSE`;

function getBackend(): OcttyBackend {
  if (!backend) {
    throw new Error("Backend not initialized");
  }
  return backend;
}

function broadcastEvent(event: unknown): void {
  for (const window of windows) {
    if (!window.isDestroyed()) {
      window.webContents.send(OCTTY_EVENT_CHANNEL, event);
    }
  }
}

function sendAppShortcut(action: AppShortcutAction): void {
  const targetWindow = BrowserWindow.getFocusedWindow() ?? [...windows][0] ?? null;
  if (!targetWindow || targetWindow.isDestroyed()) {
    return;
  }
  targetWindow.webContents.send("octty:shortcut", action);
}

function menuTargetWindow(): BrowserWindow | null {
  const targetWindow = BrowserWindow.getFocusedWindow() ?? [...windows][0] ?? null;
  return targetWindow && !targetWindow.isDestroyed() ? targetWindow : null;
}

function showAboutDialog(): void {
  const message = `Octty ${app.getVersion()}`;
  const detail = [
    `Electron ${process.versions.electron}`,
    `Chromium ${process.versions.chrome}`,
    `Node.js ${process.versions.node}`,
  ].join("\n");
  const window = menuTargetWindow();
  const options = {
    type: "info" as const,
    title: "About Octty",
    message,
    detail,
    buttons: ["OK"],
  };

  if (window) {
    void dialog.showMessageBox(window, options);
    return;
  }
  void dialog.showMessageBox(options);
}

function openExternalUrl(url: string): void {
  void shell.openExternal(url);
}

function workspaceMenuLabel(workspace: WorkspaceSummary): string {
  if (workspace.projectDisplayName) {
    return `${workspace.displayName} (${workspace.projectDisplayName})`;
  }
  return workspace.displayName;
}

function buildWorkspaceMenuItems(): MenuItemConstructorOptions[] {
  let targets: ReturnType<typeof workspaceShortcutTargets> = [];
  try {
    const payload = getBackend().getBootstrap();
    targets = workspaceShortcutTargets(payload.projectRoots, payload.workspaces);
  } catch {
    targets = [];
  }

  if (targets.length === 0) {
    return [
      {
        label: "No Workspaces",
        enabled: false,
      },
    ];
  }

  return targets.map(({ workspace, index }) => ({
    label: workspaceMenuLabel(workspace),
    accelerator: workspaceShortcutAccelerator(index),
    click: () => {
      const action = workspaceShortcutActionForIndex(index);
      if (action) {
        sendAppShortcut(action);
      }
    },
  }));
}

function installApplicationMenu(): void {
  const paneMenuItems: MenuItemConstructorOptions[] = [
    {
      label: "New Shell",
      accelerator: "Ctrl+Shift+S",
      click: () => sendAppShortcut("open-shell-pane"),
    },
    {
      label: "New Codex",
      accelerator: "Ctrl+Shift+A",
      click: () => sendAppShortcut("open-codex-pane"),
    },
    {
      label: "New Pi",
      accelerator: "Ctrl+Shift+P",
      click: () => sendAppShortcut("open-pi-pane"),
    },
    {
      label: "New Nvim",
      accelerator: "Ctrl+Shift+N",
      click: () => sendAppShortcut("open-nvim-pane"),
    },
    {
      label: "New JJUI",
      accelerator: "Ctrl+Shift+J",
      click: () => sendAppShortcut("open-jjui-pane"),
    },
    {
      label: "New Browser",
      accelerator: "Ctrl+Shift+B",
      click: () => sendAppShortcut("open-browser-pane"),
    },
    {
      label: "New Diff",
      accelerator: "Ctrl+Shift+D",
      click: () => sendAppShortcut("open-diff-pane"),
    },
  ];
  const template: MenuItemConstructorOptions[] = [
    {
      label: "Workspaces",
      submenu: buildWorkspaceMenuItems(),
    },
    {
      label: "Pane",
      submenu: paneMenuItems,
    },
    {
      label: "View",
      submenu: [
        { role: "reload" },
        { role: "toggleDevTools" },
      ],
    },
    {
      label: "Help",
      submenu: [
        {
          label: "About Octty",
          click: () => showAboutDialog(),
        },
        { type: "separator" },
        {
          label: "GitHub",
          click: () => openExternalUrl(GITHUB_URL),
        },
        {
          label: "License",
          click: () => openExternalUrl(LICENSE_URL),
        },
      ],
    },
  ];

  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

function refreshApplicationMenuForWorkspaceEvent(event: unknown): void {
  const type = (event as { type?: unknown }).type;
  if (type === "nav-updated") {
    installApplicationMenu();
  }
}

function registerIpcHandlers(): void {
  browserPaneManager.registerIpcHandlers();
  ipcMain.handle("octty:get-bootstrap", () => getBackend().getBootstrap());
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
  ipcMain.handle("octty:add-project-root", (_event, path: string) => getBackend().addProjectRoot(path));
  ipcMain.handle("octty:remove-project-root", (_event, rootId: string) =>
    getBackend().removeProjectRoot(rootId),
  );
  ipcMain.handle(
    "octty:update-project-root-display-name",
    (_event, rootId: string, displayName: string) =>
      getBackend().updateProjectRootDisplayName(rootId, displayName),
  );
  ipcMain.handle("octty:create-workspace", (_event, payload) => getBackend().createWorkspace(payload));
  ipcMain.handle(
    "octty:update-workspace-display-name",
    (_event, workspaceId: string, displayName: string) =>
      getBackend().updateWorkspaceDisplayName(workspaceId, displayName),
  );
  ipcMain.handle("octty:forget-workspace", (_event, workspaceId: string) =>
    getBackend().forgetWorkspace(workspaceId),
  );
  ipcMain.handle("octty:delete-and-forget-workspace", (_event, workspaceId: string) =>
    getBackend().deleteAndForgetWorkspace(workspaceId),
  );
  ipcMain.handle("octty:open-workspace", (_event, workspaceId: string, viewportWidth?: number) =>
    getBackend().openWorkspace(workspaceId, viewportWidth),
  );
  ipcMain.handle("octty:save-snapshot", (_event, workspaceId: string, snapshot) =>
    getBackend().saveSnapshot(workspaceId, snapshot),
  );
  ipcMain.handle("octty:create-note", (_event, workspaceId: string, fileName: string) =>
    getBackend().createNote(workspaceId, fileName),
  );
  ipcMain.handle("octty:save-note", (_event, workspaceId: string, notePath: string, body: string) =>
    getBackend().saveNote(workspaceId, notePath, body),
  );
  ipcMain.handle("octty:mark-note-read", (_event, workspaceId: string, notePath: string) =>
    getBackend().markNoteRead(workspaceId, notePath),
  );
  ipcMain.handle("octty:create-terminal-session", (_event, request) =>
    getBackend().createTerminalSession(request),
  );
  ipcMain.handle("octty:get-session", (_event, sessionId: string) => {
    const session = getBackend().getSession(sessionId);
    if (!session) {
      throw new Error("Session not found");
    }
    return session;
  });
  ipcMain.handle("octty:read-terminal-clipboard-paste", () => readTerminalClipboardPaste());
  ipcMain.handle("octty:open-external", (_event, url: string) => shell.openExternal(url));

  ipcMain.on("octty:terminal-input", (_event, payload) => {
    getBackend().sendTerminalInput(payload.sessionId, payload.data);
  });
  ipcMain.on("octty:terminal-resize", (_event, payload) => {
    getBackend().resizeTerminal(payload.sessionId, payload.cols, payload.rows);
  });
  ipcMain.on("octty:terminal-focus", (_event, payload) => {
    getBackend().focusTerminal(payload.sessionId, payload.focused);
  });
  ipcMain.on("octty:terminal-detach", (_event, payload) => {
    getBackend().detachTerminal(payload.sessionId);
  });
  ipcMain.on("octty:terminal-close", (_event, payload) => {
    getBackend().closeTerminal(payload.sessionId);
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
  const ownerWebContentsId = window.webContents.id;
  window.on("closed", () => {
    browserPaneManager.destroyForOwnerId(ownerWebContentsId);
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
  await app.whenReady();
  process.env.OCTTY_SOURCE_ROOT ||= app.isPackaged ? process.resourcesPath : appRoot;
  process.env.OCTTY_USER_DATA_PATH ||= app.getPath("userData");
  process.env.OCTTY_CACHE_PATH ||= join(app.getPath("sessionData"), "octty");
  await assertRuntimeDependencies();
  backend = new OcttyBackend();
  await backend.init();
  backend.onEvent((event) => {
    broadcastEvent(event);
    refreshApplicationMenuForWorkspaceEvent(event);
  });
  registerIpcHandlers();
  installApplicationMenu();
  createMainWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createMainWindow();
    }
  });
}

app.on("window-all-closed", () => {
  browserPaneManager.dispose();
  backend?.dispose();
  app.quit();
});

void main().catch((error) => {
  console.error("[electron] failed to start", error);
  if (app.isReady()) {
    dialog.showErrorBox("Octty failed to start", error instanceof Error ? error.message : String(error));
  }
  browserPaneManager.dispose();
  backend?.dispose();
  app.exit(1);
});

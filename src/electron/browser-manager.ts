import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  session,
  shell,
  View,
  WebContentsView,
  type BrowserWindowConstructorOptions,
  type DownloadItem,
  type IpcMainEvent,
  type IpcMainInvokeEvent,
  type Rectangle,
  type WebContents,
} from "electron";
import { join } from "node:path";
import {
  OCTTY_BROWSER_EVENT_CHANNEL,
} from "../shared/desktop-bridge";
import type {
  BrowserDownloadState,
  BrowserEventEnvelope,
  BrowserFindResult,
  BrowserViewBounds,
  BrowserViewState,
} from "../shared/types";
import {
  clampBrowserZoom,
} from "../shared/browser-utils";
import {
  appShortcutActionForKeyEvent,
} from "../shared/app-shortcuts";

type BrowserRuntime = {
  workspaceId: string;
  paneId: string;
  ownerId: number;
  owner: WebContents;
  container: View;
  view: WebContentsView;
  visible: boolean;
  errorText: string | null;
  lastBounds: BrowserViewBounds | null;
  destroying: boolean;
};

type BrowserEnsureRequest = {
  workspaceId: string;
  paneId: string;
  url: string;
  zoomFactor?: number;
  pendingPopupId?: string | null;
};

type PendingBrowserPopup = {
  id: string;
  workspaceId: string;
  openerPaneId: string;
  owner: WebContents;
  ownerId: number;
  container: View;
  view: WebContentsView;
  url: string;
};

type PaneRequest = {
  paneId: string;
};

type BoundsRequest = PaneRequest & {
  bounds: BrowserViewBounds;
};

type NavigateRequest = PaneRequest & {
  url: string;
};

type ZoomRequest = PaneRequest & {
  zoomFactor: number;
};

type FindRequest = PaneRequest & {
  text: string;
  options?: {
    forward?: boolean;
    findNext?: boolean;
  };
};

type StopFindRequest = PaneRequest & {
  action?: "clearSelection" | "keepSelection" | "activateSelection";
};

const BROWSER_PARTITION = "persist:octty-browser";
const EMBEDDABLE_PROTOCOLS = new Set(["http:", "https:", "about:"]);
const PROMPTED_PERMISSIONS = new Set([
  "clipboard-read",
  "geolocation",
  "media",
  "notifications",
]);

function runtimeKey(owner: WebContents, paneId: string): string {
  return `${owner.id}:${paneId}`;
}

function runtimeKeyForOwnerId(ownerId: number, paneId: string): string {
  return `${ownerId}:${paneId}`;
}

function browserWindowFor(owner: WebContents): BrowserWindow {
  const window = BrowserWindow.fromWebContents(owner);
  if (!window || window.isDestroyed()) {
    throw new Error("Browser pane owner window is unavailable");
  }
  return window;
}

function focusOwnerRenderer(runtime: BrowserRuntime): void {
  try {
    const ownerWindow = BrowserWindow.fromWebContents(runtime.owner);
    ownerWindow?.focus();
    if (!runtime.owner.isDestroyed()) {
      runtime.owner.focus();
    }
  } catch {
    // Focus handoff is best effort during window teardown.
  }
}

function isEmbeddableUrl(url: string): boolean {
  try {
    return EMBEDDABLE_PROTOCOLS.has(new URL(url).protocol);
  } catch {
    return false;
  }
}

function coerceBounds(bounds: BrowserViewBounds): BrowserViewBounds {
  return {
    x: Math.max(0, Math.round(bounds.x)),
    y: Math.max(0, Math.round(bounds.y)),
    width: Math.max(0, Math.round(bounds.width)),
    height: Math.max(0, Math.round(bounds.height)),
  };
}

function browserState(runtime: BrowserRuntime): BrowserViewState {
  const { webContents } = runtime.view;
  return {
    workspaceId: runtime.workspaceId,
    paneId: runtime.paneId,
    url: webContents.getURL(),
    title: webContents.getTitle(),
    loading: webContents.isLoading(),
    canGoBack: webContents.navigationHistory.canGoBack(),
    canGoForward: webContents.navigationHistory.canGoForward(),
    zoomFactor: webContents.getZoomFactor(),
    errorText: runtime.errorText,
  };
}

function sanitizeFindResult(result: Electron.Result): BrowserFindResult {
  return {
    requestId: result.requestId,
    activeMatchOrdinal: result.activeMatchOrdinal,
    matches: result.matches,
    finalUpdate: result.finalUpdate,
  };
}

export class BrowserPaneManager {
  private readonly runtimes = new Map<string, BrowserRuntime>();
  private readonly runtimeByWebContentsId = new Map<number, BrowserRuntime>();
  private readonly pendingPopups = new Map<string, PendingBrowserPopup>();
  private readonly permissionDecisions = new Map<string, boolean>();
  private popupCounter = 0;
  private downloadCounter = 0;
  private sessionConfigured = false;

  registerIpcHandlers(): void {
    ipcMain.handle("octty:browser-ensure", (event, request: BrowserEnsureRequest) =>
      this.ensure(event, request),
    );
    ipcMain.handle("octty:browser-focus", (event, request: PaneRequest) => {
      this.requireRuntime(event.sender, request.paneId).view.webContents.focus();
    });
    ipcMain.handle("octty:browser-navigate", (event, request: NavigateRequest) =>
      this.navigate(event, request.paneId, request.url),
    );
    ipcMain.handle("octty:browser-back", (event, request: PaneRequest) =>
      this.withNavigationAction(event, request.paneId, (runtime) => {
        if (runtime.view.webContents.navigationHistory.canGoBack()) {
          runtime.view.webContents.navigationHistory.goBack();
        }
      }),
    );
    ipcMain.handle("octty:browser-forward", (event, request: PaneRequest) =>
      this.withNavigationAction(event, request.paneId, (runtime) => {
        if (runtime.view.webContents.navigationHistory.canGoForward()) {
          runtime.view.webContents.navigationHistory.goForward();
        }
      }),
    );
    ipcMain.handle("octty:browser-reload", (event, request: PaneRequest) =>
      this.withNavigationAction(event, request.paneId, (runtime) => {
        runtime.view.webContents.reload();
      }),
    );
    ipcMain.handle("octty:browser-stop", (event, request: PaneRequest) =>
      this.withNavigationAction(event, request.paneId, (runtime) => {
        runtime.view.webContents.stop();
      }),
    );
    ipcMain.handle("octty:browser-zoom", (event, request: ZoomRequest) => {
      const runtime = this.requireRuntime(event.sender, request.paneId);
      runtime.view.webContents.setZoomFactor(clampBrowserZoom(request.zoomFactor));
      this.emitState(runtime);
      return browserState(runtime);
    });
    ipcMain.handle("octty:browser-find", (event, request: FindRequest) => {
      const runtime = this.requireRuntime(event.sender, request.paneId);
      const text = request.text.trim();
      if (!text) {
        runtime.view.webContents.stopFindInPage("clearSelection");
        return null;
      }
      const requestId = runtime.view.webContents.findInPage(text, {
        forward: request.options?.forward ?? true,
        findNext: request.options?.findNext ?? false,
      });
      return {
        requestId,
        activeMatchOrdinal: 0,
        matches: 0,
        finalUpdate: false,
      } satisfies BrowserFindResult;
    });
    ipcMain.handle("octty:browser-stop-find", (event, request: StopFindRequest) => {
      this.requireRuntime(event.sender, request.paneId).view.webContents.stopFindInPage(
        request.action ?? "clearSelection",
      );
    });
    ipcMain.handle("octty:browser-devtools", (event, request: PaneRequest) => {
      this.requireRuntime(event.sender, request.paneId).view.webContents.openDevTools({
        mode: "detach",
      });
    });

    ipcMain.on("octty:browser-bounds", (event, request: BoundsRequest) => {
      this.setBounds(event, request.paneId, request.bounds);
    });
    ipcMain.on("octty:browser-hide", (event, request: PaneRequest) => {
      this.hide(event.sender, request.paneId);
    });
    ipcMain.on("octty:browser-destroy", (event, request: PaneRequest) => {
      this.destroy(event.sender, request.paneId);
    });
  }

  dispose(): void {
    for (const runtime of Array.from(this.runtimes.values())) {
      this.destroyRuntime(runtime);
    }
    for (const pending of Array.from(this.pendingPopups.values())) {
      this.destroyPendingPopup(pending);
    }
  }

  destroyForOwner(owner: WebContents): void {
    let ownerId: number;
    try {
      ownerId = owner.id;
    } catch {
      return;
    }
    this.destroyForOwnerId(ownerId);
  }

  destroyForOwnerId(ownerId: number): void {
    for (const runtime of Array.from(this.runtimes.values())) {
      if (runtime.ownerId === ownerId) {
        this.destroyRuntime(runtime);
      }
    }
    for (const pending of Array.from(this.pendingPopups.values())) {
      if (pending.ownerId === ownerId) {
        this.destroyPendingPopup(pending);
      }
    }
  }

  private configureSession(): void {
    if (this.sessionConfigured) {
      return;
    }

    const browserSession = session.fromPartition(BROWSER_PARTITION);
    browserSession.setPermissionRequestHandler((webContents, permission, callback, details) => {
      if (!PROMPTED_PERMISSIONS.has(permission)) {
        callback(false);
        return;
      }

      const runtime = this.runtimeByWebContentsId.get(webContents.id);
      if (!runtime) {
        callback(false);
        return;
      }

      const origin = (() => {
        try {
          return new URL(details.requestingUrl).origin;
        } catch {
          return details.requestingUrl || "this site";
        }
      })();
      const decisionKey = `${origin}:${permission}`;
      const remembered = this.permissionDecisions.get(decisionKey);
      if (remembered !== undefined) {
        callback(remembered);
        return;
      }

      void dialog
        .showMessageBox(browserWindowFor(runtime.owner), {
          type: "question",
          buttons: ["Allow", "Deny"],
          defaultId: 1,
          cancelId: 1,
          title: "Browser permission request",
          message: `${origin} wants ${permission.replace(/-/g, " ")} access.`,
          detail: "This permission is remembered until Octty exits.",
          noLink: true,
        })
        .then((result) => {
          const allowed = result.response === 0;
          this.permissionDecisions.set(decisionKey, allowed);
          callback(allowed);
        })
        .catch(() => callback(false));
    });

    browserSession.on("will-download", (_event, item, webContents) => {
      this.handleDownload(item, webContents);
    });
    this.sessionConfigured = true;
  }

  private createBrowserView(options?: {
    webPreferences?: Electron.WebPreferences;
    webContents?: WebContents;
  }): WebContentsView {
    const constructorOptions: Electron.WebContentsViewConstructorOptions = {
      webPreferences: {
        ...options?.webPreferences,
        partition: BROWSER_PARTITION,
        javascript: true,
        nodeIntegration: false,
        contextIsolation: true,
        sandbox: true,
      },
    };
    if (options?.webContents) {
      constructorOptions.webContents = options.webContents;
    }
    const view = new WebContentsView(constructorOptions);
    view.setBackgroundColor("#ffffff");
    return view;
  }

  private async ensure(
    event: IpcMainInvokeEvent,
    request: BrowserEnsureRequest,
  ): Promise<BrowserViewState> {
    this.configureSession();
    const key = runtimeKey(event.sender, request.paneId);
    const existing = this.runtimes.get(key);
    if (existing) {
      existing.workspaceId = request.workspaceId;
      existing.view.webContents.setZoomFactor(clampBrowserZoom(request.zoomFactor ?? existing.view.webContents.getZoomFactor()));
      this.emitState(existing);
      return browserState(existing);
    }

    const ownerWindow = browserWindowFor(event.sender);
    const pendingPopup = request.pendingPopupId
      ? this.pendingPopups.get(request.pendingPopupId)
      : null;
    const container = pendingPopup?.container ?? new View();
    const view = pendingPopup?.view ?? this.createBrowserView();
    if (pendingPopup) {
      this.pendingPopups.delete(pendingPopup.id);
    } else {
      container.addChildView(view);
      container.setVisible(false);
      ownerWindow.contentView.addChildView(container);
    }

    const runtime: BrowserRuntime = {
      workspaceId: request.workspaceId,
      paneId: request.paneId,
      ownerId: event.sender.id,
      owner: event.sender,
      container,
      view,
      visible: false,
      errorText: null,
      lastBounds: null,
      destroying: false,
    };
    this.runtimes.set(key, runtime);
    this.runtimeByWebContentsId.set(view.webContents.id, runtime);
    this.bindWebContents(runtime);
    view.webContents.setZoomFactor(clampBrowserZoom(request.zoomFactor ?? 1));

    if (pendingPopup) {
      this.emitState(runtime);
    } else if (isEmbeddableUrl(request.url)) {
      await view.webContents.loadURL(request.url);
    } else {
      await shell.openExternal(request.url);
    }
    return browserState(runtime);
  }

  private bindWebContents(runtime: BrowserRuntime): void {
    const { webContents } = runtime.view;

    const update = () => {
      runtime.errorText = null;
      this.emitState(runtime);
    };

    webContents.on("did-start-loading", update);
    webContents.on("did-stop-loading", update);
    webContents.on("did-navigate", update);
    webContents.on("did-navigate-in-page", update);
    webContents.on("page-title-updated", (event) => {
      event.preventDefault();
      update();
    });
    webContents.on("did-fail-load", (_event, errorCode, errorDescription, validatedUrl, isMainFrame) => {
      if (!isMainFrame || errorCode === -3) {
        return;
      }
      runtime.errorText = `${errorDescription} (${validatedUrl})`;
      this.emitState(runtime);
    });
    webContents.on("focus", () => {
      this.emit(runtime, {
        type: "focus",
        payload: {
          workspaceId: runtime.workspaceId,
          paneId: runtime.paneId,
        },
      });
    });
    webContents.on("found-in-page", (_event, result) => {
      this.emit(runtime, {
        type: "find-result",
        payload: {
          workspaceId: runtime.workspaceId,
          paneId: runtime.paneId,
          result: sanitizeFindResult(result),
        },
      });
    });
    webContents.on("before-input-event", (event, input) => {
      const ctrlOrMeta = input.control || input.meta;
      if (!ctrlOrMeta) {
        return;
      }
      const appShortcut = appShortcutActionForKeyEvent({
        key: input.key,
        ctrlKey: input.control,
        shiftKey: input.shift,
        altKey: input.alt,
        metaKey: input.meta,
      });
      if (appShortcut) {
        event.preventDefault();
        focusOwnerRenderer(runtime);
        runtime.owner.send("octty:shortcut", appShortcut);
        return;
      }
      const key = input.key.toLowerCase();
      if (key === "l" && !input.shift && !input.alt) {
        event.preventDefault();
        focusOwnerRenderer(runtime);
        this.emit(runtime, {
          type: "shortcut",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
            action: "focus-location",
          },
        });
        return;
      }
      if (key === "f" && !input.shift && !input.alt) {
        event.preventDefault();
        focusOwnerRenderer(runtime);
        this.emit(runtime, {
          type: "shortcut",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
            action: "focus-find",
          },
        });
        return;
      }
      if (key === "r" && !input.shift && !input.alt) {
        event.preventDefault();
        runtime.view.webContents.reload();
        return;
      }
      if ((key === "+" || key === "=") && !input.alt) {
        event.preventDefault();
        this.emit(runtime, {
          type: "shortcut",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
            action: "zoom-in",
          },
        });
        return;
      }
      if (key === "-" && !input.shift && !input.alt) {
        event.preventDefault();
        this.emit(runtime, {
          type: "shortcut",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
            action: "zoom-out",
          },
        });
        return;
      }
      if (key === "0" && !input.shift && !input.alt) {
        event.preventDefault();
        this.emit(runtime, {
          type: "shortcut",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
            action: "zoom-reset",
          },
        });
        return;
      }
      if (key === "w" && input.shift) {
        event.preventDefault();
        focusOwnerRenderer(runtime);
        runtime.owner.send("octty:shortcut", "close-pane");
      }
    });
    webContents.setWindowOpenHandler(({ url }) => {
      if (!isEmbeddableUrl(url)) {
        void shell.openExternal(url);
        return { action: "deny" };
      }
      const popupId = `popup-${Date.now()}-${++this.popupCounter}`;
      return {
        action: "allow",
        overrideBrowserWindowOptions: {
          webPreferences: {
            partition: BROWSER_PARTITION,
            javascript: true,
            nodeIntegration: false,
            contextIsolation: true,
            sandbox: true,
          },
        },
        createWindow: (options) => {
          const popupWebContents = (options as BrowserWindowConstructorOptions & {
            webContents?: WebContents;
          }).webContents;
          const ownerWindow = browserWindowFor(runtime.owner);
          const container = new View();
          const popupView = this.createBrowserView({
            webContents: popupWebContents,
            webPreferences: options.webPreferences,
          });
          container.addChildView(popupView);
          container.setVisible(false);
          ownerWindow.contentView.addChildView(container);
          this.pendingPopups.set(popupId, {
            id: popupId,
            workspaceId: runtime.workspaceId,
            openerPaneId: runtime.paneId,
            owner: runtime.owner,
            ownerId: runtime.ownerId,
            container,
            view: popupView,
            url,
          });
          popupView.webContents.once("destroyed", () => {
            const pending = this.pendingPopups.get(popupId);
            if (pending) {
              this.destroyPendingPopup(pending);
            }
          });
          this.emit(runtime, {
            type: "popup",
            payload: {
              workspaceId: runtime.workspaceId,
              paneId: runtime.paneId,
              popupId,
              url,
              title: null,
            },
          });
          return popupWebContents ?? popupView.webContents;
        },
      };
    });
    webContents.on("will-navigate", (event, url) => {
      if (isEmbeddableUrl(url)) {
        return;
      }
      event.preventDefault();
      void shell.openExternal(url);
    });
    webContents.on("destroyed", () => {
      this.runtimeByWebContentsId.delete(webContents.id);
      this.runtimes.delete(runtimeKeyForOwnerId(runtime.ownerId, runtime.paneId));
      try {
        const ownerWindow = BrowserWindow.fromWebContents(runtime.owner);
        ownerWindow?.contentView.removeChildView(runtime.container);
      } catch {
        // The owner may already be gone.
      }
      if (!runtime.destroying) {
        this.emit(runtime, {
          type: "close",
          payload: {
            workspaceId: runtime.workspaceId,
            paneId: runtime.paneId,
          },
        });
      }
    });
  }

  private async navigate(
    event: IpcMainInvokeEvent,
    paneId: string,
    url: string,
  ): Promise<BrowserViewState> {
    const runtime = this.requireRuntime(event.sender, paneId);
    runtime.errorText = null;
    if (!isEmbeddableUrl(url)) {
      await shell.openExternal(url);
      return browserState(runtime);
    }
    await runtime.view.webContents.loadURL(url);
    return browserState(runtime);
  }

  private async withNavigationAction(
    event: IpcMainInvokeEvent,
    paneId: string,
    action: (runtime: BrowserRuntime) => void,
  ): Promise<BrowserViewState> {
    const runtime = this.requireRuntime(event.sender, paneId);
    action(runtime);
    this.emitState(runtime);
    return browserState(runtime);
  }

  private setBounds(event: IpcMainEvent, paneId: string, bounds: BrowserViewBounds): void {
    const runtime = this.runtimes.get(runtimeKey(event.sender, paneId));
    if (!runtime) {
      return;
    }

    const next = coerceBounds(bounds);
    if (next.width <= 0 || next.height <= 0) {
      this.hide(event.sender, paneId);
      return;
    }

    const content = runtime.view.webContents.isDestroyed()
      ? next
      : coerceBounds(bounds.contentBounds ?? next);
    const childBounds = {
      x: content.x - next.x,
      y: content.y - next.y,
      width: content.width,
      height: content.height,
    };

    browserWindowFor(event.sender).contentView.addChildView(runtime.container);
    runtime.container.setBounds(next as Rectangle);
    runtime.view.setBounds(childBounds as Rectangle);
    runtime.container.setVisible(true);
    runtime.visible = true;
    runtime.lastBounds = next;
  }

  private hide(owner: WebContents, paneId: string): void {
    const runtime = this.runtimes.get(runtimeKey(owner, paneId));
    if (!runtime) {
      return;
    }
    try {
      if (runtime.view.webContents.isFocused()) {
        focusOwnerRenderer(runtime);
      }
    } catch {
      // The embedded contents may already be torn down.
    }
    runtime.container.setVisible(false);
    runtime.visible = false;
  }

  private destroy(owner: WebContents, paneId: string): void {
    const runtime = this.runtimes.get(runtimeKey(owner, paneId));
    if (runtime) {
      this.destroyRuntime(runtime);
    }
  }

  private destroyRuntime(runtime: BrowserRuntime): void {
    runtime.destroying = true;
    this.runtimes.delete(runtimeKeyForOwnerId(runtime.ownerId, runtime.paneId));
    try {
      this.runtimeByWebContentsId.delete(runtime.view.webContents.id);
    } catch {
      // The embedded contents may already be destroyed during app/window teardown.
    }
    try {
      const ownerWindow = BrowserWindow.fromWebContents(runtime.owner);
      ownerWindow?.contentView.removeChildView(runtime.container);
    } catch {
      // The owning window may already be gone.
    }
    try {
      if (!runtime.view.webContents.isDestroyed()) {
        runtime.view.webContents.close({ waitForBeforeUnload: false });
      }
    } catch {
      // The native view can be torn down by Electron before our explicit cleanup.
    }
  }

  private destroyPendingPopup(pending: PendingBrowserPopup): void {
    this.pendingPopups.delete(pending.id);
    try {
      const ownerWindow = BrowserWindow.fromWebContents(pending.owner);
      ownerWindow?.contentView.removeChildView(pending.container);
    } catch {
      // The owner may already be gone.
    }
    try {
      if (!pending.view.webContents.isDestroyed()) {
        pending.view.webContents.close({ waitForBeforeUnload: false });
      }
    } catch {
      // The popup may already be gone.
    }
  }

  private requireRuntime(owner: WebContents, paneId: string): BrowserRuntime {
    const runtime = this.runtimes.get(runtimeKey(owner, paneId));
    if (!runtime) {
      throw new Error("Browser pane is not initialized");
    }
    return runtime;
  }

  private handleDownload(item: DownloadItem, webContents: WebContents): void {
    const runtime = this.runtimeByWebContentsId.get(webContents.id);
    if (!runtime) {
      return;
    }

    const id = `download-${Date.now()}-${++this.downloadCounter}`;
    item.setSaveDialogOptions({
      title: "Save download",
      defaultPath: join(app.getPath("downloads"), item.getFilename()),
    });

    const emitDownload = (state: BrowserDownloadState["state"]) => {
      this.emit(runtime, {
        type: "download",
        payload: {
          workspaceId: runtime.workspaceId,
          download: {
            id,
            paneId: runtime.paneId,
            fileName: item.getFilename(),
            receivedBytes: item.getReceivedBytes(),
            totalBytes: item.getTotalBytes(),
            state,
            savePath: item.getSavePath() || null,
          },
        },
      });
    };

    item.on("updated", (_event, state) => {
      emitDownload(state === "interrupted" ? "interrupted" : "progressing");
    });
    item.once("done", (_event, state) => {
      emitDownload(state);
    });
    emitDownload("progressing");
  }

  private emitState(runtime: BrowserRuntime): void {
    this.emit(runtime, {
      type: "state",
      payload: browserState(runtime),
    });
  }

  private emit(runtime: BrowserRuntime, event: BrowserEventEnvelope): void {
    try {
      if (runtime.owner.isDestroyed()) {
        return;
      }
      runtime.owner.send(OCTTY_BROWSER_EVENT_CHANNEL, event);
    } catch {
      return;
    }
  }
}

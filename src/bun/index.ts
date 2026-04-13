import Electrobun, { BrowserWindow, GlobalShortcut, PATHS } from "electrobun/bun";
import { native, toCString } from "../../node_modules/electrobun/dist-linux-x64/api/bun/proc/native";
import type {
  CreateNotePayload,
  OpenWorkspacePayload,
  CreateProjectRootPayload,
  CreateWorkspacePayload,
  MarkNoteReadPayload,
  PickDirectoryPayload,
  SaveNotePayload,
  TerminalCreateRequest,
  UpdateDisplayNamePayload,
  WorkspaceSnapshotPayload,
} from "../shared/types";
import { WorkspaceService } from "./service";
import { createShutdownController } from "./shutdown";

type WebSocketData = {
  removeClient?: () => void;
};

const service = new WorkspaceService();
await service.init();

const DEBUG_MESSAGE_RATES =
  process.env.OCTTY_DEBUG_MESSAGE_RATES === "1" ||
  process.env.WORKSPACE_ORBIT_DEBUG_MESSAGE_RATES === "1";
const OPEN_DEVTOOLS_ON_START =
  process.env.OCTTY_OPEN_DEVTOOLS === "1" ||
  process.env.WORKSPACE_ORBIT_OPEN_DEVTOOLS === "1";

type DebugRateBucket = {
  count: number;
  sample?: Record<string, unknown>;
};

const mainDebugRateBuckets = new Map<string, DebugRateBucket>();
let mainDebugRateTimer: Timer | null = null;

function trackMainDebugRate(key: string, sample?: Record<string, unknown>): void {
  if (!DEBUG_MESSAGE_RATES) {
    return;
  }

  const bucket = mainDebugRateBuckets.get(key) ?? { count: 0 };
  bucket.count += 1;
  if (sample) {
    bucket.sample = sample;
  }
  mainDebugRateBuckets.set(key, bucket);

  if (mainDebugRateTimer) {
    return;
  }

  mainDebugRateTimer = setTimeout(() => {
    mainDebugRateTimer = null;
    if (mainDebugRateBuckets.size === 0) {
      return;
    }

    const summary = Array.from(mainDebugRateBuckets.entries())
      .sort((left, right) => right[1].count - left[1].count)
      .map(([bucketKey, value]) => ({
        key: bucketKey,
        count: value.count,
        sample: value.sample,
      }));
    mainDebugRateBuckets.clear();
    console.log("[debug-rates][main]", summary);
  }, 1_000);
}

function withCors(response: Response): Response {
  const headers = new Headers(response.headers);
  headers.set("Access-Control-Allow-Origin", "*");
  headers.set("Access-Control-Allow-Headers", "Content-Type");
  headers.set("Access-Control-Allow-Methods", "GET,POST,PUT,DELETE,OPTIONS");
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

function json(data: unknown, status = 200): Response {
  return withCors(
    Response.json(data, {
      status,
    }),
  );
}

function noContent(): Response {
  return withCors(new Response(null, { status: 204 }));
}

async function readJson<T>(request: Request): Promise<T> {
  return (await request.json()) as T;
}

async function readOptionalJson<T>(request: Request): Promise<T | null> {
  const text = await request.text();
  if (!text.trim()) {
    return null;
  }
  return JSON.parse(text) as T;
}

function notFound(): Response {
  return json({ error: "Not found" }, 404);
}

const server = Bun.serve<WebSocketData>({
  port: 0,
  fetch(request, instance) {
    const url = new URL(request.url);
    trackMainDebugRate(`http:${request.method} ${url.pathname}`);

    if (request.method === "OPTIONS") {
      return noContent();
    }

    if (url.pathname === "/ws") {
      const upgraded = instance.upgrade(request, {
        data: {},
      });
      return upgraded ? undefined : json({ error: "WebSocket upgrade failed" }, 400);
    }

    return (async () => {
      try {
        if (request.method === "GET" && url.pathname === "/api/bootstrap") {
          return json(service.getBootstrap());
        }

        if (request.method === "POST" && url.pathname === "/api/dialog/directory") {
          const payload = await readJson<PickDirectoryPayload>(request);
          return json({
            path: await service.pickDirectory(payload.startingFolder),
          });
        }

        if (request.method === "POST" && url.pathname === "/api/project-roots") {
          const payload = await readJson<CreateProjectRootPayload>(request);
          return json(await service.addProjectRoot(payload.path), 201);
        }

        if (request.method === "DELETE" && url.pathname.startsWith("/api/project-roots/")) {
          const rootId = decodeURIComponent(url.pathname.replace("/api/project-roots/", ""));
          await service.removeProjectRoot(rootId);
          return noContent();
        }

        if (request.method === "PUT" && url.pathname.endsWith("/display-name") && url.pathname.startsWith("/api/project-roots/")) {
          const rootId = decodeURIComponent(
            url.pathname.replace("/api/project-roots/", "").replace("/display-name", ""),
          );
          const payload = await readJson<UpdateDisplayNamePayload>(request);
          return json(await service.updateProjectRootDisplayName(rootId, payload.displayName));
        }

        if (request.method === "POST" && url.pathname === "/api/workspaces") {
          const payload = await readJson<CreateWorkspacePayload>(request);
          return json(await service.createWorkspace(payload), 201);
        }

        if (request.method === "PUT" && url.pathname.endsWith("/display-name") && url.pathname.startsWith("/api/workspaces/")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/display-name", ""),
          );
          const payload = await readJson<UpdateDisplayNamePayload>(request);
          return json(await service.updateWorkspaceDisplayName(workspaceId, payload.displayName));
        }

        if (request.method === "POST" && url.pathname.endsWith("/delete-and-forget") && url.pathname.startsWith("/api/workspaces/")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/delete-and-forget", ""),
          );
          await service.deleteAndForgetWorkspace(workspaceId);
          return noContent();
        }

        if (request.method === "DELETE" && url.pathname.startsWith("/api/workspaces/")) {
          const workspaceId = decodeURIComponent(url.pathname.replace("/api/workspaces/", ""));
          await service.forgetWorkspace(workspaceId);
          return noContent();
        }

        if (request.method === "POST" && url.pathname.endsWith("/open")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/open", ""),
          );
          const payload = await readOptionalJson<OpenWorkspacePayload>(request);
          return json(await service.openWorkspace(workspaceId, payload?.viewportWidth));
        }

        if (request.method === "POST" && url.pathname.endsWith("/snapshot")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/snapshot", ""),
          );
          const payload = await readJson<WorkspaceSnapshotPayload>(request);
          return json(await service.saveSnapshot(workspaceId, payload.snapshot));
        }

        if (request.method === "POST" && url.pathname.endsWith("/notes")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/notes", ""),
          );
          const payload = await readJson<CreateNotePayload>(request);
          return json(await service.createNote(workspaceId, payload.fileName), 201);
        }

        if (request.method === "PUT" && url.pathname.endsWith("/notes")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/notes", ""),
          );
          const payload = await readJson<SaveNotePayload>(request);
          return json(await service.saveNote(workspaceId, payload.path, payload.body));
        }

        if (request.method === "POST" && url.pathname.endsWith("/notes/read")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/notes/read", ""),
          );
          const payload = await readJson<MarkNoteReadPayload>(request);
          await service.markNoteRead(workspaceId, payload.path);
          return noContent();
        }

        if (request.method === "POST" && url.pathname.endsWith("/sessions")) {
          const workspaceId = decodeURIComponent(
            url.pathname.replace("/api/workspaces/", "").replace("/sessions", ""),
          );
          const payload = await readJson<Omit<TerminalCreateRequest, "workspaceId">>(request);
          return json(
            await service.createTerminalSession({
              workspaceId,
              ...payload,
            }),
            201,
          );
        }

        if (request.method === "GET" && url.pathname.startsWith("/api/sessions/")) {
          const sessionId = decodeURIComponent(url.pathname.replace("/api/sessions/", ""));
          const session = service.getSession(sessionId);
          if (!session) {
            return json({ error: "Session not found" }, 404);
          }
          return json(session);
        }

        return notFound();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        return json({ error: message }, 500);
      }
    })();
  },
  websocket: {
    open(ws) {
      ws.data.removeClient = service.addClient((message) => {
        ws.send(JSON.stringify(message));
      });
      ws.send(
        JSON.stringify({
          type: "nav-updated",
          payload: service.getBootstrap(),
        }),
      );
    },
    close(ws) {
      ws.data.removeClient?.();
    },
    message(_ws, rawMessage) {
      const text =
        typeof rawMessage === "string"
          ? rawMessage
          : Buffer.from(rawMessage).toString("utf8");
      const message = JSON.parse(text) as
        | { type: "terminal-input"; payload: { sessionId: string; data: string } }
        | { type: "terminal-resize"; payload: { sessionId: string; cols: number; rows: number } }
        | { type: "terminal-focus"; payload: { sessionId: string; focused: boolean } }
        | { type: "terminal-detach"; payload: { sessionId: string } }
        | { type: "terminal-close"; payload: { sessionId: string } }
        | {
            type: "terminal-ui-debug";
            payload: { message: string; details?: Record<string, unknown> };
          };

      trackMainDebugRate(`ws-in:${message.type}`, (() => {
        if (message.type === "terminal-resize") {
          return {
            sessionId: message.payload.sessionId,
            cols: message.payload.cols,
            rows: message.payload.rows,
          };
        }
        if (
          message.type === "terminal-focus" ||
          message.type === "terminal-detach" ||
          message.type === "terminal-close"
        ) {
          return {
            sessionId: message.payload.sessionId,
          };
        }
        if (message.type === "terminal-input") {
          return {
            sessionId: message.payload.sessionId,
            bytes: message.payload.data.length,
          };
        }
        return undefined;
      })());

      if (message.type === "terminal-input") {
        service.writeToSession(message.payload.sessionId, message.payload.data);
      }

      if (message.type === "terminal-resize") {
        service.resizeSession(
          message.payload.sessionId,
          message.payload.cols,
          message.payload.rows,
        );
      }

      if (message.type === "terminal-focus") {
        service.setSessionFocused(message.payload.sessionId, message.payload.focused);
      }

      if (message.type === "terminal-detach") {
        service.detachSession(message.payload.sessionId);
      }

      if (message.type === "terminal-close") {
        service.closeSession(message.payload.sessionId);
      }

      if (
        message.type === "terminal-ui-debug" &&
        (process.env.OCTTY_DEBUG_TERMINAL === "1" ||
          process.env.WORKSPACE_ORBIT_DEBUG_TERMINAL === "1" ||
          DEBUG_MESSAGE_RATES)
      ) {
        console.log("[terminal-ui]", message.payload.message, message.payload.details ?? {});
      }
    },
  },
});

const shutdown = createShutdownController({
  unregisterShortcuts: () => GlobalShortcut.unregisterAll(),
  stopServer: () => server.stop(true),
  disposeService: () => service.dispose(),
  exit: (code = 0) => process.exit(code),
});

// Electrobun already quits automatically when the last window closes. Running our
// cleanup from before-quit keeps app teardown out of the native window-destroy path.
Electrobun.events.on("before-quit", () => {
  shutdown.shutdown(false);
});

const apiOrigin = `http://127.0.0.1:${server.port}`;
const debugTerminal =
  process.env.OCTTY_DEBUG_TERMINAL === "1" || process.env.WORKSPACE_ORBIT_DEBUG_TERMINAL === "1"
    ? "1"
    : "0";
const debugMessageRates = DEBUG_MESSAGE_RATES ? "1" : "0";
const ghosttyRenderLoopMode =
  process.env.OCTTY_GHOSTTY_RENDER_LOOP_MODE ??
  process.env.WORKSPACE_ORBIT_GHOSTTY_RENDER_LOOP_MODE ??
  "throttled";
const ghosttyRenderIntervalMs = String(
  Math.max(
    16,
    Number.parseInt(
      process.env.OCTTY_GHOSTTY_RENDER_INTERVAL_MS ??
        process.env.WORKSPACE_ORBIT_GHOSTTY_RENDER_INTERVAL_MS ??
        "80",
      10,
    ) || 80,
  ),
);
const headlessApi =
  process.env.OCTTY_HEADLESS_API === "1" || process.env.WORKSPACE_ORBIT_HEADLESS_API === "1";
const bootstrapHtml = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="octty-api-origin" content="${apiOrigin}" />
    <meta name="octty-debug-terminal" content="${debugTerminal}" />
    <meta name="octty-debug-message-rates" content="${debugMessageRates}" />
    <meta name="octty-ghostty-render-loop-mode" content="${ghosttyRenderLoopMode}" />
    <meta name="octty-ghostty-render-interval-ms" content="${ghosttyRenderIntervalMs}" />
    <title>Octty</title>
    <style>
      :root {
        color-scheme: light dark;
        font-family: system-ui, sans-serif;
        --boot-bg: #f5f7fb;
        --boot-fg: #17202b;
        --boot-error: #9e2f2f;
      }
      @media (prefers-color-scheme: dark) {
        :root {
          --boot-bg: #0c0f13;
          --boot-fg: #e7edf4;
          --boot-error: #ffb8b8;
        }
      }
      html, body {
        width: 100%;
        height: 100%;
        margin: 0;
        overflow: hidden;
        background: var(--boot-bg);
      }
      body {
        background: var(--boot-bg);
        color: var(--boot-fg);
        font-family: system-ui, sans-serif;
        line-height: 1.4;
      }
      #root {
        position: fixed;
        inset: 0;
        display: flex;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
        background: var(--boot-bg);
      }
      #root[data-boot="loading"] {
        display: grid;
        place-items: center;
        opacity: 0.9;
      }
      .boot-error {
        white-space: pre-wrap;
        padding: 24px;
        color: var(--boot-error);
      }
    </style>
    <link rel="stylesheet" href="views://mainview/index.css" />
  </head>
  <body>
    <div id="root" data-boot="loading">Loading Octty...</div>
    <script>
      (function () {
        const root = document.getElementById("root");
        const showError = function (label, value) {
          const text = value && value.stack ? value.stack : String(value);
          if (root) {
            root.className = "boot-error";
            root.textContent = label + "\\n\\n" + text;
          }
        };
        window.addEventListener("error", function (event) {
          showError("Renderer error", event.error || event.message);
        });
        window.addEventListener("unhandledrejection", function (event) {
          showError("Unhandled promise rejection", event.reason);
        });
      })();
    </script>
    <script src="views://mainview/index.js"></script>
  </body>
</html>`;

if (headlessApi) {
  console.log(`[octty] headless api ready at ${apiOrigin}`);
} else {
  const mainWindow = new BrowserWindow({
    title: "Octty",
    renderer: "cef",
    frame: {
      x: 80,
      y: 48,
      width: 1600,
      height: 1024,
    },
    url: null,
    html: bootstrapHtml,
    viewsRoot: PATHS.VIEWS_FOLDER,
  });

  const relayoutTimers = new Set<Timer>();
  let lastWebviewFrame = "";
  const syncMainWebviewFrame = () => {
    const frame = mainWindow.getFrame();
    const frameKey = `${frame.width}:${frame.height}`;
    if (frameKey === lastWebviewFrame) {
      return;
    }

    lastWebviewFrame = frameKey;
    native.symbols.resizeWebview(
      mainWindow.webview.ptr,
      0,
      0,
      frame.width,
      frame.height,
      toCString("[]"),
    );
  };
  const scheduleMainWebviewSync = (delayMs: number) => {
    const timer = setTimeout(() => {
      relayoutTimers.delete(timer);
      syncMainWebviewFrame();
    }, delayMs);
    relayoutTimers.add(timer);
  };

  for (const delayMs of [0, 80, 220, 500, 1000]) {
    scheduleMainWebviewSync(delayMs);
  }

  if (OPEN_DEVTOOLS_ON_START) {
    const timer = setTimeout(() => {
      relayoutTimers.delete(timer);
      mainWindow.webview.openDevTools();
    }, 900);
    relayoutTimers.add(timer);
  }

  // Keep only the window-close overrides and native-only actions on Electrobun's
  // native/global shortcut layer. Arrow-based pane/workspace navigation stays in
  // the renderer because the current native shortcut path is unreliable for those
  // chords under Hyprland.
  const nativeAppShortcuts = [
    ["CommandOrControl+W", "block-window-close"],
    ["CommandOrControl+Shift+W", "close-pane"],
    ["CommandOrControl+Shift+I", "toggle-devtools"],
  ] as const;

  const invokeRendererShortcut = (action: (typeof nativeAppShortcuts)[number][1]) => {
    if (action === "toggle-devtools") {
      mainWindow.webview.toggleDevTools();
      return;
    }

    mainWindow.webview.executeJavascript(`
      window.dispatchEvent(
        new CustomEvent("octty-shortcut", { detail: ${JSON.stringify(action)} })
      );
    `);
  };

  const registerAppShortcuts = () => {
    for (const [accelerator, action] of nativeAppShortcuts) {
      if (GlobalShortcut.isRegistered(accelerator)) {
        continue;
      }
      GlobalShortcut.register(accelerator, () => invokeRendererShortcut(action));
    }
  };

  const unregisterAppShortcuts = () => {
    for (const [accelerator] of nativeAppShortcuts) {
      if (!GlobalShortcut.isRegistered(accelerator)) {
        continue;
      }
      GlobalShortcut.unregister(accelerator);
    }
  };

  registerAppShortcuts();
  Electrobun.events.on(`close-${mainWindow.id}`, () => {
    for (const timer of relayoutTimers) {
      clearTimeout(timer);
    }
    relayoutTimers.clear();
  });
}

process.on("SIGINT", () => {
  shutdown.shutdown();
});

process.on("SIGTERM", () => {
  shutdown.shutdown();
});

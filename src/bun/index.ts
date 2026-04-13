import Electrobun, { BrowserWindow, GlobalShortcut, PATHS } from "electrobun/bun";
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
          process.env.WORKSPACE_ORBIT_DEBUG_TERMINAL === "1")
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

const apiOrigin = `http://127.0.0.1:${server.port}`;
const debugTerminal =
  process.env.OCTTY_DEBUG_TERMINAL === "1" || process.env.WORKSPACE_ORBIT_DEBUG_TERMINAL === "1"
    ? "1"
    : "0";
const headlessApi =
  process.env.OCTTY_HEADLESS_API === "1" || process.env.WORKSPACE_ORBIT_HEADLESS_API === "1";
const bootstrapHtml = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="octty-api-origin" content="${apiOrigin}" />
    <meta name="octty-debug-terminal" content="${debugTerminal}" />
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

  // These are registered through Electrobun's native/global shortcut layer on purpose.
  // The embedded native browser webview can take focus in a way that bypasses the
  // renderer's DOM key handlers, so renderer-only shortcuts become unreliable whenever
  // a browser pane is visible or active. Routing the app's navigation/layout shortcuts
  // through the native bridge keeps pane/workspace control responsive across shell,
  // note, diff, and browser panes until Electrobun exposes a window-scoped shortcut
  // path that still fires while the native child webview owns focus.
  const appShortcuts = [
    ["CommandOrControl+W", "block-window-close"],
    ["CommandOrControl+Shift+W", "close-pane"],
    ["CommandOrControl+Shift+Left", "focus-pane-left"],
    ["CommandOrControl+Shift+Right", "focus-pane-right"],
    ["CommandOrControl+Shift+Up", "focus-workspace-up"],
    ["CommandOrControl+Shift+Down", "focus-workspace-down"],
    ["CommandOrControl+Alt+Left", "resize-pane-left"],
    ["CommandOrControl+Alt+Right", "resize-pane-right"],
    ["CommandOrControl+Alt+Shift+Left", "move-pane-left"],
    ["CommandOrControl+Alt+Shift+Right", "move-pane-right"],
  ] as const;

  const invokeRendererShortcut = (action: (typeof appShortcuts)[number][1]) => {
    mainWindow.webview.executeJavascript(`
      if (typeof window.__workspaceOrbitInvokeShortcut === "function") {
        window.__workspaceOrbitInvokeShortcut(${JSON.stringify(action)});
      } else if (${JSON.stringify(action)} === "close-pane" && typeof window.__workspaceOrbitHandleClosePane === "function") {
        window.__workspaceOrbitHandleClosePane();
      }
    `);
  };

  const registerAppShortcuts = () => {
    for (const [accelerator, action] of appShortcuts) {
      if (GlobalShortcut.isRegistered(accelerator)) {
        continue;
      }
      GlobalShortcut.register(accelerator, () => invokeRendererShortcut(action));
    }
  };

  const unregisterAppShortcuts = () => {
    for (const [accelerator] of appShortcuts) {
      if (!GlobalShortcut.isRegistered(accelerator)) {
        continue;
      }
      GlobalShortcut.unregister(accelerator);
    }
  };

  registerAppShortcuts();
  Electrobun.events.on(`close-${mainWindow.id}`, () => {
    unregisterAppShortcuts();
    shutdown.shutdown();
  });
}

process.on("SIGINT", () => {
  shutdown.shutdown();
});

process.on("SIGTERM", () => {
  shutdown.shutdown();
});

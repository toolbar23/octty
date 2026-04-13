import { Electroview } from "electrobun/view";
import { init as initGhostty, Terminal, FitAddon, type ITheme } from "ghostty-web";
import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createRoot } from "react-dom/client";
import {
  addPane,
  findPaneColumnId,
  moveColumn,
  moveColumnToCenter,
  movePaneHorizontally,
  movePaneToColumn,
  movePaneToNewColumn,
  pinColumn,
  removePane,
  resizePaneColumn,
  setActivePane,
  setColumnHeightFractions,
  setColumnWidth,
  updatePane,
} from "../shared/layout";
import type {
  AgentAttentionState,
  BootstrapPayload,
  BrowserPanePayload,
  NoteRecord,
  NotePanePayload,
  PaneState,
  PaneType,
  SessionSnapshot,
  TerminalKind,
  TerminalPanePayload,
  WorkspaceColumn,
  WorkspaceDetail,
  WorkspaceState,
  WorkspaceSnapshot,
  WorkspaceSummary,
} from "../shared/types";
import {
  agentAttentionClassName,
  agentAttentionLabel,
} from "../shared/agent-attention";
import {
  hasRecordedWorkspacePath,
} from "../shared/types";
import {
  BufferedStringFlusher,
  shouldFlushTerminalInputImmediately,
  takeStringChunk,
} from "../shared/terminal-batching";
import { shouldRemapShiftEnterToCtrlJ } from "../shared/terminal-shortcuts";
import {
  isAgentTerminalKind,
  supportsTerminalAttention,
  terminalKindLabel,
} from "../shared/terminal-kind";
import {
  workspaceStateClassName,
  workspaceStateLabel,
} from "../shared/workspace-state";

const apiOrigin =
  document
    .querySelector('meta[name="octty-api-origin"], meta[name="workspace-orbit-api-origin"]')
    ?.getAttribute("content") ??
  new URLSearchParams(window.location.search).get("apiOrigin") ??
  "http://127.0.0.1:0";
const debugTerminal =
  document.querySelector('meta[name="octty-debug-terminal"], meta[name="workspace-orbit-debug-terminal"]')?.getAttribute("content") ===
  "1";
const wsOrigin = apiOrigin.replace("http://", "ws://").replace("https://", "wss://");
let ghosttyInitPromise: Promise<void> | null = null;
let forwardTerminalUiDebug:
  | ((message: string, details: Record<string, unknown>) => void)
  | null = null;
const MAX_TERMINAL_WRITE_CHUNK = 16_384;
const MAX_TERMINAL_WRITE_CHUNKS_PER_TICK = 4;
const TERMINAL_INPUT_BATCH_DELAY_MS = 4;
const TERMINAL_INPUT_BATCH_SIZE = 256;
const TERMINAL_REQUEST_TIMEOUT_MS = 8_000;
const TERMINAL_TOOLBAR_KINDS: TerminalKind[] = ["shell", "codex", "pi", "nvim", "jjui"];
const TASKSPACE_DRAG_MIME = "application/x-octty-layout";
const MIN_STACK_PANE_HEIGHT_PX = 96;
const KEYBOARD_COLUMN_RESIZE_STEP_PX = 80;
const CTRL_J = "\x0a";

function isElectrobunRuntime(): boolean {
  const electrobunWindow = window as Window & {
    __electrobun?: unknown;
    __electrobunRpcSocketPort?: unknown;
    __electrobunWebviewId?: unknown;
  };

  return (
    typeof electrobunWindow.__electrobun === "object" &&
    typeof electrobunWindow.__electrobunRpcSocketPort === "number" &&
    typeof electrobunWindow.__electrobunWebviewId === "number"
  );
}

type WorkspaceOrbitWindow = Window & {
  __workspaceOrbitHandleClosePane?: () => void;
  __workspaceOrbitInvokeShortcut?: (action: string) => void;
};

if (isElectrobunRuntime()) {
  new Electroview({ rpc: null as any });
}

async function ensureGhostty(): Promise<void> {
  ghosttyInitPromise ||= initGhostty();
  await ghosttyInitPromise;
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${apiOrigin}${path}`, {
    ...init,
    headers: {
      "Content-Type": "application/json",
      ...(init?.headers ?? {}),
    },
  });

  if (!response.ok) {
    const data = (await response.json().catch(() => null)) as { error?: string } | null;
    throw new Error(data?.error || `Request failed: ${response.status}`);
  }

  if (response.status === 204) {
    return undefined as T;
  }

  return (await response.json()) as T;
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
  let timer = 0;
  const timeout = new Promise<never>((_, reject) => {
    timer = window.setTimeout(() => {
      reject(new Error(`${label} timed out after ${Math.round(timeoutMs / 1000)}s`));
    }, timeoutMs);
  });

  try {
    return await Promise.race([promise, timeout]);
  } finally {
    window.clearTimeout(timer);
  }
}

async function copyTextToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }

  const textarea = document.createElement("textarea");
  textarea.value = text;
  textarea.setAttribute("readonly", "true");
  textarea.style.position = "fixed";
  textarea.style.top = "0";
  textarea.style.left = "0";
  textarea.style.opacity = "0";
  document.body.appendChild(textarea);
  textarea.focus();
  textarea.select();

  try {
    if (!document.execCommand("copy")) {
      throw new Error("Copy command was rejected");
    }
  } finally {
    document.body.removeChild(textarea);
  }
}

type SessionEvent =
  | { type: "output"; data: string }
  | { type: "exit"; exitCode: number | null };

type KeyboardNavigationRequest = {
  workspaceId: string;
  paneId: string | null;
  nonce: number;
};

const paneFocusRegistry = new Map<string, () => boolean>();

type TerminalRuntime = {
  paneId: string;
  host: HTMLDivElement;
  term: Terminal;
  fitAddon: FitAddon;
  sessionId: string | null;
  unsubscribe: (() => void) | null;
  onExit: (exitCode: number | null) => void;
  sendInput: (sessionId: string, data: string) => void;
  enqueueInput: (data: string) => void;
  flushInput: () => void;
  setSessionId: (sessionId: string | null) => void;
  resizeSession: (sessionId: string, cols: number, rows: number) => void;
  enqueueWrite: (data: string) => void;
  clearPendingOutput: () => void;
};

const terminalRuntimeRegistry = new Map<string, TerminalRuntime>();
const windowTimerScheduler = {
  schedule(callback: () => void, delayMs: number): number {
    return window.setTimeout(callback, delayMs);
  },
  cancel(handle: number): void {
    window.clearTimeout(handle);
  },
};

function getCssThemeColor(name: string, fallback: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim() || fallback;
}

function getTerminalTheme(): ITheme {
  return {
    background: getCssThemeColor("--color-terminal-bg", "#101317"),
    foreground: getCssThemeColor("--color-terminal-fg", "#e7edf4"),
    cursor: getCssThemeColor("--color-terminal-cursor", "#7fb0ff"),
    cursorAccent: getCssThemeColor("--color-terminal-bg", "#101317"),
    selectionBackground: getCssThemeColor("--color-terminal-selection-bg", "rgb(127 176 255 / 18%)"),
    selectionForeground: getCssThemeColor("--color-terminal-selection-fg", "#e7edf4"),
    black: getCssThemeColor("--color-terminal-black", "#32344a"),
    red: getCssThemeColor("--color-terminal-red", "#f7768e"),
    green: getCssThemeColor("--color-terminal-green", "#9ece6a"),
    yellow: getCssThemeColor("--color-terminal-yellow", "#e0af68"),
    blue: getCssThemeColor("--color-terminal-blue", "#7aa2f7"),
    magenta: getCssThemeColor("--color-terminal-magenta", "#ad8ee6"),
    cyan: getCssThemeColor("--color-terminal-cyan", "#449dab"),
    white: getCssThemeColor("--color-terminal-white", "#787c99"),
    brightBlack: getCssThemeColor("--color-terminal-bright-black", "#444b6a"),
    brightRed: getCssThemeColor("--color-terminal-bright-red", "#ff7a93"),
    brightGreen: getCssThemeColor("--color-terminal-bright-green", "#b9f27c"),
    brightYellow: getCssThemeColor("--color-terminal-bright-yellow", "#ff9e64"),
    brightBlue: getCssThemeColor("--color-terminal-bright-blue", "#7da6ff"),
    brightMagenta: getCssThemeColor("--color-terminal-bright-magenta", "#bb9af7"),
    brightCyan: getCssThemeColor("--color-terminal-bright-cyan", "#0db9d7"),
    brightWhite: getCssThemeColor("--color-terminal-bright-white", "#acb0d0"),
  };
}

function applyTerminalTheme(term: Terminal): void {
  const theme = getTerminalTheme();
  term.options.theme = theme;
  term.renderer?.setTheme(theme);
}

function refreshTerminalThemes(): void {
  for (const runtime of terminalRuntimeRegistry.values()) {
    applyTerminalTheme(runtime.term);
  }
}

if (typeof window.matchMedia === "function") {
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", refreshTerminalThemes);
}

function formatTerminalChunk(data: string, limit = 120): string {
  const encoded = JSON.stringify(data);
  if (encoded.length <= limit) {
    return encoded;
  }
  return `${encoded.slice(0, limit)}...`;
}

function createTerminalRuntime(paneId: string): TerminalRuntime {
  const host = document.createElement("div");
  host.className = "terminal-runtime-host";
  host.style.display = "flex";
  host.style.flex = "1 1 auto";
  host.style.minWidth = "0";
  host.style.minHeight = "0";
  host.style.width = "100%";
  host.style.height = "100%";

  const term = new Terminal({
    fontSize: 15,
    fontFamily: "JetBrains Mono, monospace",
    theme: getTerminalTheme(),
  });
  const fitAddon = new FitAddon();
  term.loadAddon(fitAddon);
  term.open(host);
  applyTerminalTheme(term);
  term.attachCustomKeyEventHandler((event) => {
    if (debugTerminal) {
      logTerminalUi("ghostty-keydown", () => ({
        paneId,
        key: event.key,
        code: event.code,
        repeat: event.repeat,
        isComposing: event.isComposing,
        ctrlKey: event.ctrlKey,
        altKey: event.altKey,
        shiftKey: event.shiftKey,
        metaKey: event.metaKey,
        target: describeElement(event.target),
        activeElement: describeElement(document.activeElement),
      }));
    }

    if (shouldRemapShiftEnterToCtrlJ(event)) {
      logTerminalUi("term-remap-shift-enter", () => ({
        paneId,
        sessionId: runtime.sessionId,
      }));
      runtime.enqueueInput(CTRL_J);
      return true;
    }

    return false;
  });
  host.tabIndex = 0;
  host.setAttribute("spellcheck", "false");
  scrubTerminalSurface(host);
  fitAddon.fit();
  fitAddon.observeResize();

  const writeQueue: string[] = [];
  let writeDrainScheduled = false;
  const drainWriteQueue = () => {
    let processedChunks = 0;
    while (processedChunks < MAX_TERMINAL_WRITE_CHUNKS_PER_TICK) {
      const chunk = takeStringChunk(writeQueue, MAX_TERMINAL_WRITE_CHUNK);
      if (!chunk) {
        return;
      }
      term.write(chunk);
      processedChunks += 1;
    }

    if (writeQueue.length > 0) {
      window.setTimeout(scheduleWriteDrain, 0);
    }
  };
  const scheduleWriteDrain = () => {
    if (writeDrainScheduled) {
      return;
    }
    writeDrainScheduled = true;
    queueMicrotask(() => {
      writeDrainScheduled = false;
      drainWriteQueue();
    });
  };

  const runtime: TerminalRuntime = {
    paneId,
    host,
    term,
    fitAddon,
    sessionId: null,
    unsubscribe: null,
    onExit: () => {},
    sendInput: () => {},
    enqueueInput(data: string) {
      if (!runtime.sessionId) {
        logTerminalUi("term-drop-data", () => ({
          paneId,
          sessionId: runtime.sessionId,
          data: formatTerminalChunk(data),
        }));
        return;
      }
      inputBatcher.add(data);
    },
    flushInput() {
      inputBatcher.flush();
    },
    setSessionId(sessionId: string | null) {
      if (runtime.sessionId === sessionId) {
        return;
      }
      inputBatcher.flush();
      runtime.sessionId = sessionId;
    },
    resizeSession: () => {},
    enqueueWrite(data: string) {
      if (!data) {
        return;
      }
      writeQueue.push(data);
      scheduleWriteDrain();
    },
    clearPendingOutput() {
      writeQueue.length = 0;
    },
  };
  const inputBatcher = new BufferedStringFlusher<number>({
    flushDelayMs: TERMINAL_INPUT_BATCH_DELAY_MS,
    maxBatchSize: TERMINAL_INPUT_BATCH_SIZE,
    scheduler: windowTimerScheduler,
    shouldFlushImmediately: shouldFlushTerminalInputImmediately,
    onFlush(data) {
      if (!runtime.sessionId) {
        logTerminalUi("term-drop-data", () => ({
          paneId,
          sessionId: runtime.sessionId,
          data: formatTerminalChunk(data),
        }));
        return;
      }
      runtime.sendInput(runtime.sessionId, data);
    },
  });

  term.onData((data) => {
    logTerminalUi("term-on-data", () => ({
      paneId,
      sessionId: runtime.sessionId,
      data: formatTerminalChunk(data),
      activeElement: describeElement(document.activeElement),
    }));
    runtime.enqueueInput(data);
  });

  term.onResize(({ cols, rows }) => {
    logTerminalUi("term-on-resize", () => ({
      paneId,
      sessionId: runtime.sessionId,
      cols,
      rows,
    }));
    if (runtime.sessionId && runtime.host.isConnected) {
      runtime.resizeSession(runtime.sessionId, cols, rows);
    }
  });

  terminalRuntimeRegistry.set(paneId, runtime);
  return runtime;
}

function getOrCreateTerminalRuntime(paneId: string): TerminalRuntime {
  return terminalRuntimeRegistry.get(paneId) ?? createTerminalRuntime(paneId);
}

function attachTerminalRuntime(runtime: TerminalRuntime, container: HTMLDivElement): void {
  if (container.firstChild !== runtime.host) {
    container.replaceChildren(runtime.host);
  }
  requestAnimationFrame(() => {
    if (runtime.host.isConnected) {
      runtime.fitAddon.fit();
    }
  });
}

function refitConnectedTerminalRuntimes(): void {
  for (const runtime of terminalRuntimeRegistry.values()) {
    if (!runtime.host.isConnected) {
      continue;
    }
    runtime.fitAddon.fit();
  }
}

function detachTerminalRuntime(runtime: TerminalRuntime): void {
  const activeElement = document.activeElement;
  if (activeElement instanceof HTMLElement && runtime.host.contains(activeElement)) {
    activeElement.blur();
  }
  runtime.host.remove();
}

function resetTerminalRuntime(runtime: TerminalRuntime): void {
  runtime.clearPendingOutput();
  runtime.term.reset();
}

function bindTerminalRuntimeSession(
  runtime: TerminalRuntime,
  sessionId: string,
  subscribeSession: (sessionId: string, listener: (event: SessionEvent) => void) => () => void,
): void {
  if (runtime.sessionId === sessionId && runtime.unsubscribe) {
    return;
  }

  runtime.unsubscribe?.();
  runtime.setSessionId(sessionId);
  runtime.unsubscribe = subscribeSession(sessionId, (event) => {
    if (event.type === "output") {
      logTerminalUi("session-output", () => ({
        paneId: runtime.paneId,
        sessionId,
        data: formatTerminalChunk(event.data),
        activeElement: describeElement(document.activeElement),
      }));
      runtime.enqueueWrite(event.data);
      return;
    }

    runtime.setSessionId(null);
    runtime.unsubscribe?.();
    runtime.unsubscribe = null;
    runtime.onExit(event.exitCode);
  });
}

function destroyTerminalRuntime(paneId: string): void {
  const runtime = terminalRuntimeRegistry.get(paneId);
  if (!runtime) {
    return;
  }
  runtime.unsubscribe?.();
  runtime.unsubscribe = null;
  runtime.setSessionId(null);
  runtime.clearPendingOutput();
  runtime.host.remove();
  runtime.fitAddon.dispose();
  runtime.term.dispose();
  terminalRuntimeRegistry.delete(paneId);
}

function describeElement(target: EventTarget | null): string {
  if (!(target instanceof Element)) {
    return String(target);
  }

  const tag = target.tagName.toLowerCase();
  const id = target.id ? `#${target.id}` : "";
  const className =
    typeof target.className === "string" && target.className.trim().length > 0
      ? `.${target.className.trim().replace(/\s+/g, ".")}`
      : "";
  const active = target === document.activeElement ? "[active]" : "";
  return `${tag}${id}${className}${active}`;
}

function logTerminalUi(
  message: string,
  details: Record<string, unknown> | (() => Record<string, unknown>) = {},
): void {
  if (!debugTerminal) {
    return;
  }

  const resolvedDetails = typeof details === "function" ? details() : details;
  console.log("[terminal-ui]", message, resolvedDetails);
  if (
    message !== "session-output" &&
    message !== "term-on-resize" &&
    message !== "socket-send" &&
    message !== "socket-drop"
  ) {
    forwardTerminalUiDebug?.(message, resolvedDetails);
  }
}

function summarizeSocketMessage(message: unknown): Record<string, unknown> {
  if (!message || typeof message !== "object") {
    return { message };
  }

  const data = message as { type?: unknown; payload?: Record<string, unknown> | null };
  const payload = data.payload ?? {};

  if (data.type === "terminal-input") {
    return {
      type: data.type,
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
      data:
        typeof payload.data === "string" ? formatTerminalChunk(payload.data) : String(payload.data),
    };
  }

  if (data.type === "terminal-resize") {
    return {
      type: data.type,
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
      cols: typeof payload.cols === "number" ? payload.cols : null,
      rows: typeof payload.rows === "number" ? payload.rows : null,
    };
  }

  if (data.type === "terminal-focus") {
    return {
      type: data.type,
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
      focused: typeof payload.focused === "boolean" ? payload.focused : null,
    };
  }

  if (data.type === "terminal-detach") {
    return {
      type: data.type,
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
    };
  }

  if (data.type === "terminal-close") {
    return {
      type: data.type,
      sessionId: typeof payload.sessionId === "string" ? payload.sessionId : null,
    };
  }

  return {
    type: typeof data.type === "string" ? data.type : String(data.type ?? "unknown"),
  };
}

type LayoutDragPayload =
  | { type: "column"; columnId: string }
  | { type: "pane"; paneId: string };

function setLayoutDragPayload(
  event: React.DragEvent<HTMLElement>,
  payload: LayoutDragPayload,
): void {
  event.dataTransfer.effectAllowed = "move";
  event.dataTransfer.setData(TASKSPACE_DRAG_MIME, JSON.stringify(payload));
  event.dataTransfer.setData("text/plain", JSON.stringify(payload));
}

function getLayoutDragPayload(
  event: React.DragEvent<HTMLElement>,
): LayoutDragPayload | null {
  const raw = event.dataTransfer.getData(TASKSPACE_DRAG_MIME) || event.dataTransfer.getData("text/plain");
  if (!raw) {
    return null;
  }

  try {
    const parsed = JSON.parse(raw) as Partial<LayoutDragPayload>;
    if (parsed.type === "column" && typeof parsed.columnId === "string") {
      return { type: "column", columnId: parsed.columnId };
    }
    if (parsed.type === "pane" && typeof parsed.paneId === "string") {
      return { type: "pane", paneId: parsed.paneId };
    }
  } catch {
    return null;
  }

  return null;
}

function terminalStatusLabel(payload: TerminalPanePayload): string {
  if (payload.sessionState === "live") {
    return "live";
  }
  if (payload.autoStart) {
    return "starting";
  }
  if (payload.exitCode !== null) {
    return `exit ${payload.exitCode}`;
  }
  return "inactive";
}

function workspaceStateDescription(state: WorkspaceState): string {
  switch (state) {
    case "published":
      return "effective change is reachable from a remote bookmark";
    case "merged-local":
      return "effective change is already contained in another local workspace";
    case "draft":
      return "effective change is still unique to this workspace";
    case "conflicted":
      return "effective change has unresolved conflicts";
    case "unknown":
      return "workspace state is unavailable";
  }
}

function workspaceStateTitle(workspace: WorkspaceSummary): string {
  const lines = [
    `${workspaceStateLabel(workspace.workspaceState)}: ${workspaceStateDescription(workspace.workspaceState)}`,
    `effective change: +${workspace.effectiveAddedLines}/-${workspace.effectiveRemovedLines}`,
    workspace.hasWorkingCopyChanges
      ? "working copy has changes"
      : "no working-copy changes",
  ];
  return lines.join("\n");
}

function workspaceStateBeadText(workspace: WorkspaceSummary): string {
  const label =
    workspace.workspaceState === "merged-local"
      ? "Merged"
      : workspace.workspaceState === "conflicted"
        ? "Conflict"
        : workspaceStateLabel(workspace.workspaceState);
  if (
    workspace.workspaceState === "draft" ||
    workspace.workspaceState === "merged-local"
  ) {
    return `${label} +${workspace.effectiveAddedLines}/-${workspace.effectiveRemovedLines}`;
  }
  return label;
}

function workspaceBookmarkLabel(workspace: WorkspaceSummary): string {
  const names = workspace.bookmarks.join(", ");
  if (workspace.bookmarkRelation === "above") {
    return `${names} (+)`;
  }
  return names;
}

function currentViewportWidth(): number {
  return Math.max(window.innerWidth || 0, 1);
}

function currentViewportSize(): { width: number; height: number } {
  const docElement = document.documentElement;
  const body = document.body;
  const visualViewport = window.visualViewport;

  const width = Math.max(
    window.innerWidth || 0,
    docElement?.clientWidth || 0,
    body?.clientWidth || 0,
    visualViewport?.width || 0,
    window.outerWidth || 0,
    1,
  );
  const height = Math.max(
    window.innerHeight || 0,
    docElement?.clientHeight || 0,
    body?.clientHeight || 0,
    visualViewport?.height || 0,
    window.outerHeight || 0,
    1,
  );

  return {
    width: Math.round(width),
    height: Math.round(height),
  };
}

function listWorkspaceShortcutIds(workspaces: WorkspaceSummary[]): string[] {
  return workspaces
    .filter((workspace) => hasRecordedWorkspacePath(workspace.workspacePath))
    .map((workspace) => workspace.id);
}

function listPaneShortcutIds(snapshot: WorkspaceSnapshot): string[] {
  const orderedColumnIds = [
    ...(snapshot.pinnedLeftColumnId ? [snapshot.pinnedLeftColumnId] : []),
    ...snapshot.centerColumnIds,
    ...(snapshot.pinnedRightColumnId ? [snapshot.pinnedRightColumnId] : []),
  ];

  return orderedColumnIds.flatMap((columnId) => snapshot.columns[columnId]?.paneIds ?? []);
}

function cycleShortcutTarget(ids: string[], currentId: string | null, step: -1 | 1): string | null {
  if (ids.length === 0) {
    return null;
  }

  const currentIndex = currentId ? ids.indexOf(currentId) : -1;
  if (currentIndex === -1) {
    return step > 0 ? ids[0]! : ids[ids.length - 1]!;
  }

  const nextIndex = Math.max(0, Math.min(ids.length - 1, currentIndex + step));
  return ids[nextIndex]!;
}

function focusPaneTarget(
  paneId: string,
  paneElement: HTMLElement,
  pane: PaneState,
): boolean {
  paneElement.scrollIntoView({ block: "nearest", inline: "nearest" });

  const registeredFocus = paneFocusRegistry.get(paneId);
  if (registeredFocus) {
    return registeredFocus();
  }

  const selector =
    pane.type === "shell"
      ? "textarea, .terminal-runtime-host, .terminal-surface"
      : pane.type === "note"
        ? "textarea"
        : pane.type === "browser"
          ? "input, button"
          : "button, textarea, input, [contenteditable='true'], [tabindex]";
  const target = paneElement.querySelector<HTMLElement>(selector) ?? paneElement;
  try {
    target.focus({ preventScroll: true });
  } catch {
    target.focus();
  }
  return document.activeElement === target;
}

function mergeWorkspace(
  workspaces: WorkspaceSummary[],
  workspace: WorkspaceSummary,
): WorkspaceSummary[] {
  return workspaces.map((item) => (item.id === workspace.id ? workspace : item));
}

function App(): React.ReactElement {
  const [bootstrap, setBootstrap] = useState<BootstrapPayload>({
    projectRoots: [],
    workspaces: [],
  });
  const [details, setDetails] = useState<Record<string, WorkspaceDetail>>({});
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadingWorkspaceId, setLoadingWorkspaceId] = useState<string | null>(null);
  const [showRootForm, setShowRootForm] = useState(false);
  const [showWorkspaceForm, setShowWorkspaceForm] = useState(false);
  const [rootPathInput, setRootPathInput] = useState("");
  const [workspaceRootId, setWorkspaceRootId] = useState<string>("");
  const [workspacePathInput, setWorkspacePathInput] = useState("");
  const [workspaceNameInput, setWorkspaceNameInput] = useState("");
  const [keyboardNavigationRequest, setKeyboardNavigationRequest] =
    useState<KeyboardNavigationRequest | null>(null);

  const detailsRef = useRef(details);
  const sessionListenersRef = useRef<Map<string, Set<(event: SessionEvent) => void>>>(new Map());
  const snapshotTimersRef = useRef<Map<string, number>>(new Map());
  const socketRef = useRef<WebSocket | null>(null);
  const activeWorkspaceIdRef = useRef<string | null>(null);
  const activeDetail = activeWorkspaceId ? details[activeWorkspaceId] ?? null : null;

  useEffect(() => {
    detailsRef.current = details;
  }, [details]);

  useEffect(() => {
    let disposed = false;
    const frameIds = new Set<number>();
    const timeoutIds = new Set<number>();
    const syncViewportSize = () => {
      if (disposed) {
        return;
      }
      const { width, height } = currentViewportSize();
      document.documentElement.style.setProperty("--app-width", `${width}px`);
      document.documentElement.style.setProperty("--app-height", `${height}px`);
    };

    const scheduleAnimationSync = (count: number) => {
      for (let index = 0; index < count; index += 1) {
        const frameId = window.requestAnimationFrame(() => {
          frameIds.delete(frameId);
          syncViewportSize();
        });
        frameIds.add(frameId);
      }
    };

    syncViewportSize();
    scheduleAnimationSync(8);
    for (const delay of [0, 50, 120, 240, 500, 900]) {
      const timeoutId = window.setTimeout(() => {
        timeoutIds.delete(timeoutId);
        syncViewportSize();
      }, delay);
      timeoutIds.add(timeoutId);
    }

    const handleResize = () => {
      syncViewportSize();
    };

    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(() => {
            syncViewportSize();
          });
    resizeObserver?.observe(document.documentElement);
    if (document.body) {
      resizeObserver?.observe(document.body);
    }

    window.addEventListener("resize", handleResize);
    window.visualViewport?.addEventListener("resize", handleResize);

    return () => {
      disposed = true;
      for (const frameId of frameIds) {
        window.cancelAnimationFrame(frameId);
      }
      for (const timeoutId of timeoutIds) {
        window.clearTimeout(timeoutId);
      }
      resizeObserver?.disconnect();
      window.removeEventListener("resize", handleResize);
      window.visualViewport?.removeEventListener("resize", handleResize);
    };
  }, []);

  useEffect(() => {
    activeWorkspaceIdRef.current = activeWorkspaceId;
  }, [activeWorkspaceId]);

  const subscribeSession = useCallback(
    (sessionId: string, listener: (event: SessionEvent) => void) => {
      const listeners = sessionListenersRef.current.get(sessionId) ?? new Set();
      listeners.add(listener);
      sessionListenersRef.current.set(sessionId, listeners);

      return () => {
        const current = sessionListenersRef.current.get(sessionId);
        if (!current) {
          return;
        }
        current.delete(listener);
        if (current.size === 0) {
          sessionListenersRef.current.delete(sessionId);
        }
      };
    },
    [],
  );

  const emitSession = useCallback((sessionId: string, event: SessionEvent) => {
    const listeners = sessionListenersRef.current.get(sessionId);
    if (!listeners) {
      return;
    }
    for (const listener of listeners) {
      listener(event);
    }
  }, []);

  const scheduleSnapshotSave = useCallback((workspaceId: string, snapshot: WorkspaceSnapshot) => {
    const existing = snapshotTimersRef.current.get(workspaceId);
    if (existing) {
      window.clearTimeout(existing);
    }

    const timer = window.setTimeout(() => {
      void apiFetch<WorkspaceSnapshot>(`/api/workspaces/${encodeURIComponent(workspaceId)}/snapshot`, {
        method: "POST",
        body: JSON.stringify({ snapshot }),
      }).catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
    }, 220);
    snapshotTimersRef.current.set(workspaceId, timer);
  }, []);

  const mutateWorkspace = useCallback(
    (
      workspaceId: string,
      updater: (detail: WorkspaceDetail) => WorkspaceDetail,
    ) => {
      setDetails((current) => {
        const detail = current[workspaceId];
        if (!detail) {
          return current;
        }

        const nextDetail = updater(detail);
        scheduleSnapshotSave(workspaceId, nextDetail.snapshot);
        return {
          ...current,
          [workspaceId]: nextDetail,
        };
      });
    },
    [scheduleSnapshotSave],
  );

  const patchPane = useCallback(
    (
      workspaceId: string,
      paneId: string,
      updater: (pane: PaneState) => PaneState,
    ) => {
      mutateWorkspace(workspaceId, (detail) => ({
        ...detail,
        snapshot: updatePane(detail.snapshot, paneId, updater),
      }));
    },
    [mutateWorkspace],
  );

  const loadWorkspace = useCallback(
    async (workspaceId: string) => {
      const hasCachedDetail = Boolean(detailsRef.current[workspaceId]);
      setActiveWorkspaceId(workspaceId);
      setLoadingWorkspaceId(hasCachedDetail ? null : workspaceId);
      setError(null);
      try {
        const detail = await apiFetch<WorkspaceDetail>(
          `/api/workspaces/${encodeURIComponent(workspaceId)}/open`,
          {
            method: "POST",
            body: JSON.stringify({ viewportWidth: currentViewportWidth() }),
          },
        );
        setDetails((current) => ({
          ...current,
          [workspaceId]: detail,
        }));
        setBootstrap((current) => ({
          ...current,
          workspaces: mergeWorkspace(current.workspaces, detail.workspace),
        }));
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        setLoadingWorkspaceId((current) => (current === workspaceId ? null : current));
      }
    },
    [],
  );

  useEffect(() => {
    void apiFetch<BootstrapPayload>("/api/bootstrap")
      .then((payload) => {
        setBootstrap(payload);
        if (payload.workspaces.length > 0) {
          setWorkspaceRootId(payload.projectRoots[0]?.id ?? "");
        }
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, []);

  useEffect(() => {
    const socket = new WebSocket(`${wsOrigin}/ws`);
    socketRef.current = socket;
    const forwarder = (message: string, details: Record<string, unknown>) => {
      if (socket.readyState !== WebSocket.OPEN) {
        return;
      }
      socket.send(
        JSON.stringify({
          type: "terminal-ui-debug",
          payload: { message, details },
        }),
      );
    };
    forwardTerminalUiDebug = forwarder;
    socket.addEventListener("open", () => {
      logTerminalUi("socket-open", {});
    });
    socket.addEventListener("close", (event) => {
      logTerminalUi("socket-close", { code: event.code, reason: event.reason });
    });
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data) as { type: string; payload: unknown };

      if (message.type === "nav-updated") {
        const payload = message.payload as BootstrapPayload;
        setBootstrap(payload);
        if (!activeWorkspaceIdRef.current && payload.workspaces.length > 0) {
          setWorkspaceRootId(payload.projectRoots[0]?.id ?? "");
        }
        return;
      }

      if (message.type === "workspace-status") {
        const payload = message.payload as {
          workspace: WorkspaceSummary;
          notes: NoteRecord[] | null;
        };
        setBootstrap((current) => ({
          ...current,
          workspaces: mergeWorkspace(current.workspaces, payload.workspace),
        }));

        setDetails((current) => {
          const detail = current[payload.workspace.id];
          if (!detail) {
            return current;
          }
          return {
            ...current,
            [payload.workspace.id]: {
              ...detail,
              workspace: payload.workspace,
              notes: payload.notes ?? detail.notes,
            },
          };
        });
        return;
      }

      if (message.type === "terminal-output") {
        const payload = message.payload as { sessionId: string; data: string };
        emitSession(payload.sessionId, { type: "output", data: payload.data });
        return;
      }

      if (message.type === "terminal-session-update") {
        const payload = message.payload as {
          workspaceId: string;
          paneId: string;
          sessionId: string;
          kind: TerminalKind;
          cwd: string;
          command: string;
          sessionState: "live" | "stopped" | "missing";
          exitCode: number | null;
          embeddedSession: TerminalPanePayload["embeddedSession"];
          embeddedSessionCorrelationId: string | null;
          agentAttentionState: AgentAttentionState | null;
        };
        setDetails((current) => {
          const detail = current[payload.workspaceId];
          if (!detail) {
            return current;
          }

          return {
            ...current,
            [payload.workspaceId]: {
              ...detail,
              snapshot: updatePane(detail.snapshot, payload.paneId, (pane) => {
                if (pane.type !== "shell" && pane.type !== "agent-shell") {
                  return pane;
                }

                return {
                  ...pane,
                  payload: {
                    ...(pane.payload as TerminalPanePayload),
                    sessionId: payload.sessionId,
                    sessionState: payload.sessionState,
                    kind: payload.kind,
                    command: payload.command,
                    cwd: payload.cwd,
                    exitCode: payload.exitCode,
                    autoStart: false,
                    embeddedSession: payload.embeddedSession,
                    embeddedSessionCorrelationId: payload.embeddedSessionCorrelationId,
                    agentAttentionState: payload.agentAttentionState,
                  },
                };
              }),
            },
          };
        });
        return;
      }

      if (message.type === "terminal-exit") {
        const payload = message.payload as { sessionId: string; exitCode: number | null };
        emitSession(payload.sessionId, { type: "exit", exitCode: payload.exitCode });
      }
    });

    return () => {
      if (forwardTerminalUiDebug === forwarder) {
        forwardTerminalUiDebug = null;
      }
      socketRef.current = null;
      socket.close();
    };
  }, [emitSession, wsOrigin]);

  const sendSocketMessage = useCallback((message: unknown) => {
    const socket = socketRef.current;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      logTerminalUi("socket-drop", () => ({
        readyState: socket?.readyState ?? null,
        ...summarizeSocketMessage(message),
      }));
      return;
    }
    logTerminalUi("socket-send", () => ({
      readyState: socket.readyState,
      ...summarizeSocketMessage(message),
    }));
    socket.send(JSON.stringify(message));
  }, []);

  const activeWorkspace = useMemo(
    () =>
      activeWorkspaceId
        ? bootstrap.workspaces.find((workspace) => workspace.id === activeWorkspaceId) ?? null
        : null,
    [activeWorkspaceId, bootstrap.workspaces],
  );

  const openAddWorkspace = useCallback(() => {
    setWorkspaceRootId(bootstrap.projectRoots[0]?.id ?? "");
    setShowWorkspaceForm(true);
  }, [bootstrap.projectRoots]);

  const browseRoot = useCallback(async () => {
    try {
      const data = await apiFetch<{ path: string | null }>("/api/dialog/directory", {
        method: "POST",
        body: JSON.stringify({ startingFolder: rootPathInput || undefined }),
      });
      if (data.path) {
        setRootPathInput(data.path);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [rootPathInput]);

  const browseWorkspacePath = useCallback(async () => {
    try {
      const data = await apiFetch<{ path: string | null }>("/api/dialog/directory", {
        method: "POST",
        body: JSON.stringify({ startingFolder: workspacePathInput || undefined }),
      });
      if (data.path) {
        setWorkspacePathInput(data.path);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [workspacePathInput]);

  const submitRoot = useCallback(async () => {
    try {
      await apiFetch("/api/project-roots", {
        method: "POST",
        body: JSON.stringify({ path: rootPathInput }),
      });
      setShowRootForm(false);
      setRootPathInput("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [rootPathInput]);

  const submitWorkspace = useCallback(async () => {
    try {
      await apiFetch("/api/workspaces", {
        method: "POST",
        body: JSON.stringify({
          rootId: workspaceRootId,
          destinationPath: workspacePathInput,
          workspaceName: workspaceNameInput || undefined,
        }),
      });
      setShowWorkspaceForm(false);
      setWorkspacePathInput("");
      setWorkspaceNameInput("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [workspaceNameInput, workspacePathInput, workspaceRootId]);

  const forgetWorkspace = useCallback(async (workspaceId: string) => {
    const confirmed = window.confirm("Forget this workspace from JJ and the app?");
    if (!confirmed) {
      return;
    }

    try {
      await apiFetch(`/api/workspaces/${encodeURIComponent(workspaceId)}`, {
        method: "DELETE",
      });
      setDetails((current) => {
        const next = { ...current };
        delete next[workspaceId];
        return next;
      });
      if (activeWorkspaceId === workspaceId) {
        setActiveWorkspaceId(null);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [activeWorkspaceId]);

  const removeProjectRoot = useCallback(async (rootId: string) => {
    const confirmed = window.confirm("Remove this project root from the app?");
    if (!confirmed) {
      return;
    }

    try {
      await apiFetch(`/api/project-roots/${encodeURIComponent(rootId)}`, {
        method: "DELETE",
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const createTerminalSession = useCallback(
    async (workspaceId: string, paneId: string, kind: TerminalKind, cols = 120, rows = 32) => {
      const session = await withTimeout(
        apiFetch<SessionSnapshot>(
          `/api/workspaces/${encodeURIComponent(workspaceId)}/sessions`,
          {
            method: "POST",
            body: JSON.stringify({
              paneId,
              kind,
              cols,
              rows,
            }),
          },
        ),
        TERMINAL_REQUEST_TIMEOUT_MS,
        "Terminal session start",
      );

      patchPane(workspaceId, paneId, (pane) => ({
        ...pane,
        payload: {
          ...(pane.payload as TerminalPanePayload),
          sessionId: session.id,
          sessionState: session.state,
          kind: session.kind,
          command: session.command,
          cwd: session.cwd,
          exitCode: session.exitCode,
          autoStart: false,
          embeddedSession: session.embeddedSession,
          embeddedSessionCorrelationId: session.embeddedSessionCorrelationId,
          agentAttentionState: session.agentAttentionState,
        },
      }));
      return session;
    },
    [patchPane],
  );

  const addPaneToWorkspace = useCallback(
    (
      workspaceId: string,
      type: PaneType,
      terminalKind: TerminalKind = "shell",
    ) => {
      mutateWorkspace(workspaceId, (detail) => ({
        ...detail,
        snapshot: addPane(
          detail.snapshot,
          type,
          detail.workspace.workspacePath,
          "new-column",
          terminalKind,
          currentViewportWidth(),
        ),
      }));
    },
    [mutateWorkspace],
  );

  const invokeAppShortcut = useCallback(
    (action: string) => {
      const detail = activeWorkspaceId ? detailsRef.current[activeWorkspaceId] ?? null : null;
      const activePaneId = detail?.snapshot.activePaneId ?? null;

      switch (action) {
        case "resize-pane-left":
        case "resize-pane-right": {
          if (!detail || !activePaneId) {
            return;
          }
          mutateWorkspace(detail.workspace.id, (currentDetail) => ({
            ...currentDetail,
            snapshot: resizePaneColumn(
              currentDetail.snapshot,
              activePaneId,
              action === "resize-pane-right"
                ? KEYBOARD_COLUMN_RESIZE_STEP_PX
                : -KEYBOARD_COLUMN_RESIZE_STEP_PX,
            ),
          }));
          return;
        }
        case "move-pane-left":
        case "move-pane-right": {
          if (!detail || !activePaneId) {
            return;
          }
          setKeyboardNavigationRequest({
            workspaceId: detail.workspace.id,
            paneId: activePaneId,
            nonce: Date.now(),
          });
          mutateWorkspace(detail.workspace.id, (currentDetail) => ({
            ...currentDetail,
            snapshot: movePaneHorizontally(
              currentDetail.snapshot,
              activePaneId,
              action === "move-pane-left" ? -1 : 1,
              currentViewportWidth(),
            ),
          }));
          return;
        }
        case "focus-pane-left":
        case "focus-pane-right": {
          if (!detail) {
            return;
          }

          const nextPaneId = cycleShortcutTarget(
            listPaneShortcutIds(detail.snapshot),
            detail.snapshot.activePaneId,
            action === "focus-pane-left" ? -1 : 1,
          );
          if (!nextPaneId || nextPaneId === detail.snapshot.activePaneId) {
            return;
          }

          setKeyboardNavigationRequest({
            workspaceId: detail.workspace.id,
            paneId: nextPaneId,
            nonce: Date.now(),
          });
          mutateWorkspace(detail.workspace.id, (currentDetail) => ({
            ...currentDetail,
            snapshot: setActivePane(currentDetail.snapshot, nextPaneId),
          }));
          return;
        }
        case "focus-workspace-up":
        case "focus-workspace-down": {
          const nextWorkspaceId = cycleShortcutTarget(
            listWorkspaceShortcutIds(bootstrap.workspaces),
            activeWorkspaceId,
            action === "focus-workspace-up" ? -1 : 1,
          );
          if (!nextWorkspaceId || nextWorkspaceId === activeWorkspaceId) {
            return;
          }

          setKeyboardNavigationRequest({
            workspaceId: nextWorkspaceId,
            paneId: null,
            nonce: Date.now(),
          });
          void loadWorkspace(nextWorkspaceId);
          return;
        }
        case "close-pane": {
          if (!detail || !activePaneId) {
            return;
          }
          const activePane = detail.snapshot.panes[activePaneId] ?? null;
          if (!activePane) {
            return;
          }

          if (activePane.type === "shell") {
            const payload = activePane.payload as TerminalPanePayload;
            if (payload.sessionId) {
              sendSocketMessage({
                type: "terminal-close",
                payload: { sessionId: payload.sessionId },
              });
            }
            destroyTerminalRuntime(activePane.id);
          }

          mutateWorkspace(detail.workspace.id, (currentDetail) => ({
            ...currentDetail,
            snapshot: removePane(currentDetail.snapshot, activePane.id),
          }));
          return;
        }
        default:
          return;
      }
    },
    [activeWorkspaceId, bootstrap.workspaces, loadWorkspace, mutateWorkspace, sendSocketMessage],
  );

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) {
        return;
      }
      if (showRootForm || showWorkspaceForm) {
        return;
      }
      if (!event.ctrlKey || event.metaKey) {
        return;
      }

      if (
        event.altKey &&
        !event.shiftKey &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")
      ) {
        event.preventDefault();
        event.stopPropagation();
        invokeAppShortcut(event.key === "ArrowRight" ? "resize-pane-right" : "resize-pane-left");
        return;
      }

      if (
        event.shiftKey &&
        event.altKey &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")
      ) {
        event.preventDefault();
        event.stopPropagation();
        invokeAppShortcut(event.key === "ArrowLeft" ? "move-pane-left" : "move-pane-right");
        return;
      }

      if (event.altKey) {
        return;
      }

      if (
        event.shiftKey &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")
      ) {
        event.preventDefault();
        event.stopPropagation();
        invokeAppShortcut(event.key === "ArrowLeft" ? "focus-pane-left" : "focus-pane-right");
        return;
      }

      if (event.shiftKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        event.preventDefault();
        event.stopPropagation();
        invokeAppShortcut(event.key === "ArrowUp" ? "focus-workspace-up" : "focus-workspace-down");
        return;
      }
    };

    window.addEventListener("keydown", handleShortcut, true);
    return () => {
      window.removeEventListener("keydown", handleShortcut, true);
    };
  }, [activeWorkspaceId, bootstrap.workspaces, invokeAppShortcut, loadWorkspace, mutateWorkspace, showRootForm, showWorkspaceForm]);

  useEffect(() => {
    const orbitWindow = window as WorkspaceOrbitWindow;
    orbitWindow.__workspaceOrbitHandleClosePane = () => invokeAppShortcut("close-pane");
    orbitWindow.__workspaceOrbitInvokeShortcut = invokeAppShortcut;

    const handleClosePaneKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) {
        return;
      }
      if (!event.ctrlKey || event.metaKey) {
        return;
      }
      if (event.key.toLowerCase() !== "w" && event.code !== "KeyW") {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      if (event.shiftKey && !event.altKey) {
        invokeAppShortcut("close-pane");
      }
    };

    window.addEventListener("keydown", handleClosePaneKey, true);
    return () => {
      window.removeEventListener("keydown", handleClosePaneKey, true);
      delete orbitWindow.__workspaceOrbitHandleClosePane;
      if (orbitWindow.__workspaceOrbitInvokeShortcut === invokeAppShortcut) {
        delete orbitWindow.__workspaceOrbitInvokeShortcut;
      }
    };
  }, [invokeAppShortcut]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="project-list">
          {bootstrap.projectRoots.map((root) => (
            <section key={root.id} className="project-group">
              <div className="project-header">
                <div className="project-header-copy">
                  <div className="project-title">{root.label}</div>
                </div>
                <button className="sidebar-inline-button" onClick={() => removeProjectRoot(root.id)}>
                  Remove
                </button>
              </div>

              <div className="workspace-list">
                {bootstrap.workspaces
                  .filter((workspace) => workspace.rootId === root.id)
                  .map((workspace) => (
                    <div
                      key={workspace.id}
                      className={`workspace-item ${workspace.id === activeWorkspaceId ? "selected" : ""} ${hasRecordedWorkspacePath(workspace.workspacePath) ? "" : "unavailable"}`}
                    >
                      <button
                        className="workspace-select"
                        onClick={() => {
                          if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
                            setError(
                              `JJ reports no recorded path for workspace "${workspace.workspaceName}". Forget it in JJ or reopen it from a real workspace directory.`,
                            );
                            return;
                          }
                          void loadWorkspace(workspace.id);
                        }}
                      >
                        <div className="workspace-item-row workspace-item-head">
                          <span className="workspace-name">{workspace.workspaceName}</span>
                          {agentAttentionClassName(workspace.agentAttentionState) && (
                            <span
                              className={`agent-attention-dot workspace-attention-dot ${agentAttentionClassName(workspace.agentAttentionState)}`}
                              title={agentAttentionLabel(workspace.agentAttentionState) ?? undefined}
                            />
                          )}
                        </div>
                        {workspace.bookmarks.length > 0 && (
                          <div className="workspace-item-row workspace-bookmark-row">
                            <span className="workspace-branch-info">{workspaceBookmarkLabel(workspace)}</span>
                          </div>
                        )}
                        <div className="workspace-badges">
                          {!hasRecordedWorkspacePath(workspace.workspacePath) && (
                            <span className="badge warning">missing path</span>
                          )}
                          {workspace.unreadNotes > 0 && (
                            <span className="badge">{workspace.unreadNotes} note</span>
                          )}
                          {workspace.activeAgentCount > 0 && (
                            <span className="badge">{workspace.activeAgentCount} agent</span>
                          )}
                        </div>
                      </button>
                      <div className="workspace-side-actions">
                        <span
                          className={`workspace-state-bead ${workspaceStateClassName(workspace.workspaceState)} ${workspace.hasWorkingCopyChanges ? "changed" : "unchanged"}`}
                          title={workspaceStateTitle(workspace)}
                          aria-label={workspaceStateTitle(workspace)}
                        >
                          {workspaceStateBeadText(workspace)}
                        </span>
                        <button
                          className="sidebar-inline-button inline-action"
                          onClick={() => {
                            void forgetWorkspace(workspace.id);
                          }}
                        >
                          Forget
                        </button>
                      </div>
                    </div>
                  ))}
              </div>
            </section>
          ))}

          {bootstrap.projectRoots.length === 0 && (
            <div className="empty-state">
              <p>Add a JJ repo root to discover workspaces and restore taskspaces.</p>
            </div>
          )}
        </div>

        <div className="sidebar-footer">
          <div className="sidebar-actions">
            <button onClick={() => setShowRootForm(true)}>Add repo</button>
            <button onClick={openAddWorkspace} disabled={bootstrap.projectRoots.length === 0}>
              New workspace
            </button>
          </div>
        </div>
      </aside>

      <main className="workspace-shell">
        {activeDetail && (
          <div className="workspace-toolbar workspace-toolbar-floating">
            {TERMINAL_TOOLBAR_KINDS.map((kind) => (
              <button
                key={kind}
                onClick={() => addPaneToWorkspace(activeDetail.workspace.id, "shell", kind)}
              >
                {terminalKindLabel(kind)}
              </button>
            ))}
            <button onClick={() => addPaneToWorkspace(activeDetail.workspace.id, "note")}>Note</button>
            <button onClick={() => addPaneToWorkspace(activeDetail.workspace.id, "browser")}>Browser</button>
            <button onClick={() => addPaneToWorkspace(activeDetail.workspace.id, "diff")}>Diff</button>
          </div>
        )}

        {error && <div className="error-banner">{error}</div>}

        {showRootForm && (
          <div className="modal-backdrop">
            <div className="modal">
              <h3>Add project root</h3>
              <label>
                Path
                <div className="field-row">
                  <input value={rootPathInput} onChange={(event) => setRootPathInput(event.target.value)} />
                  <button onClick={() => void browseRoot()}>Browse</button>
                </div>
              </label>
              <div className="modal-actions">
                <button onClick={() => setShowRootForm(false)}>Cancel</button>
                <button onClick={() => void submitRoot()} disabled={!rootPathInput.trim()}>
                  Add
                </button>
              </div>
            </div>
          </div>
        )}

        {showWorkspaceForm && (
          <div className="modal-backdrop">
            <div className="modal">
              <h3>Create workspace</h3>
              <label>
                Project root
                <select value={workspaceRootId} onChange={(event) => setWorkspaceRootId(event.target.value)}>
                  {bootstrap.projectRoots.map((root) => (
                    <option key={root.id} value={root.id}>
                      {root.label}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Destination path
                <div className="field-row">
                  <input
                    value={workspacePathInput}
                    onChange={(event) => setWorkspacePathInput(event.target.value)}
                  />
                  <button onClick={() => void browseWorkspacePath()}>Browse</button>
                </div>
              </label>
              <label>
                Workspace name
                <input
                  value={workspaceNameInput}
                  onChange={(event) => setWorkspaceNameInput(event.target.value)}
                />
              </label>
              <div className="modal-actions">
                <button onClick={() => setShowWorkspaceForm(false)}>Cancel</button>
                <button
                  onClick={() => void submitWorkspace()}
                  disabled={!workspaceRootId || !workspacePathInput.trim()}
                >
                  Create
                </button>
              </div>
            </div>
          </div>
        )}

        <section className="workspace-stage">
          {!activeWorkspaceId && !loadingWorkspaceId && (
            <div className="empty-stage">
              <p>Select a workspace to restore its panes, notes, browser references, and live shells.</p>
            </div>
          )}

          {activeWorkspaceId && loadingWorkspaceId === activeWorkspaceId && !activeDetail && (
            <div className="empty-stage">
              <p>Restoring workspace...</p>
            </div>
          )}

          {activeDetail && (
            <div key={activeDetail.workspace.id} className="workspace-view active">
              <WorkspaceTaskspace
                detail={activeDetail}
                isVisible
                subscribeSession={subscribeSession}
                setActivePane={(paneId) =>
                  mutateWorkspace(activeDetail.workspace.id, (currentDetail) => ({
                    ...currentDetail,
                    snapshot: setActivePane(currentDetail.snapshot, paneId),
                  }))
                }
                updatePane={(paneId, updater) =>
                  patchPane(activeDetail.workspace.id, paneId, updater)
                }
                mutateSnapshot={(updater) =>
                  mutateWorkspace(activeDetail.workspace.id, (currentDetail) => ({
                    ...currentDetail,
                    snapshot: updater(currentDetail.snapshot),
                  }))
                }
                onCreateNote={async (fileName) => {
                  const note = await apiFetch<NoteRecord>(
                    `/api/workspaces/${encodeURIComponent(activeDetail.workspace.id)}/notes`,
                    {
                      method: "POST",
                      body: JSON.stringify({ fileName }),
                    },
                  );
                  setDetails((current) => ({
                    ...current,
                    [activeDetail.workspace.id]: {
                      ...current[activeDetail.workspace.id],
                      notes: [note, ...current[activeDetail.workspace.id].notes],
                    },
                  }));
                  return note;
                }}
                onSaveNote={async (path, body) => {
                  const note = await apiFetch<NoteRecord>(
                    `/api/workspaces/${encodeURIComponent(activeDetail.workspace.id)}/notes`,
                    {
                      method: "PUT",
                      body: JSON.stringify({ path, body }),
                    },
                  );
                  setDetails((current) => ({
                    ...current,
                    [activeDetail.workspace.id]: {
                      ...current[activeDetail.workspace.id],
                      notes: current[activeDetail.workspace.id].notes.map((item) =>
                        item.path === note.path ? note : item,
                      ),
                    },
                  }));
                  return note;
                }}
                onMarkNoteRead={async (path) => {
                  await apiFetch<void>(
                    `/api/workspaces/${encodeURIComponent(activeDetail.workspace.id)}/notes/read`,
                    {
                      method: "POST",
                      body: JSON.stringify({ path }),
                    },
                  );
                }}
                onCreateSession={createTerminalSession}
                onFetchSession={async (sessionId) =>
                  apiFetch<SessionSnapshot>(`/api/sessions/${encodeURIComponent(sessionId)}`)
                }
                onSendSessionInput={(sessionId, data) =>
                  sendSocketMessage({
                    type: "terminal-input",
                    payload: { sessionId, data },
                  })
                }
                onResizeSession={(sessionId, cols, rows) =>
                  sendSocketMessage({
                    type: "terminal-resize",
                    payload: { sessionId, cols, rows },
                  })
                }
                onSetSessionFocus={(sessionId, focused) =>
                  sendSocketMessage({
                    type: "terminal-focus",
                    payload: { sessionId, focused },
                  })
                }
                onCloseSession={(sessionId) =>
                  sendSocketMessage({
                    type: "terminal-close",
                    payload: { sessionId },
                  })
                }
                onAddPane={(type, terminalKind) =>
                  addPaneToWorkspace(activeDetail.workspace.id, type, terminalKind)
                }
                keyboardNavigationRequest={keyboardNavigationRequest}
              />
            </div>
          )}
        </section>
      </main>
    </div>
  );
}

type WorkspaceTaskspaceProps = {
  detail: WorkspaceDetail;
  isVisible: boolean;
  subscribeSession: (sessionId: string, listener: (event: SessionEvent) => void) => () => void;
  setActivePane: (paneId: string | null) => void;
  updatePane: (paneId: string, updater: (pane: PaneState) => PaneState) => void;
  mutateSnapshot: (updater: (snapshot: WorkspaceSnapshot) => WorkspaceSnapshot) => void;
  onCreateNote: (fileName: string) => Promise<NoteRecord>;
  onSaveNote: (path: string, body: string) => Promise<NoteRecord>;
  onMarkNoteRead: (path: string) => Promise<void>;
  onCreateSession: (workspaceId: string, paneId: string, kind: TerminalKind, cols?: number, rows?: number) => Promise<SessionSnapshot>;
  onFetchSession: (sessionId: string) => Promise<SessionSnapshot>;
  onSendSessionInput: (sessionId: string, data: string) => void;
  onResizeSession: (sessionId: string, cols: number, rows: number) => void;
  onSetSessionFocus: (sessionId: string, focused: boolean) => void;
  onCloseSession: (sessionId: string) => void;
  onAddPane: (type: PaneType, terminalKind?: TerminalKind) => void;
  keyboardNavigationRequest: KeyboardNavigationRequest | null;
};

function WorkspaceTaskspace(props: WorkspaceTaskspaceProps): React.ReactElement {
  const {
    detail,
    isVisible,
    subscribeSession,
    setActivePane: activatePane,
    updatePane: patchPane,
    mutateSnapshot,
    onCreateNote,
    onSaveNote,
    onMarkNoteRead,
    onCreateSession,
    onFetchSession,
    onSendSessionInput,
    onResizeSession,
    onSetSessionFocus,
    onCloseSession,
    onAddPane,
    keyboardNavigationRequest,
  } = props;
  const centerScrollRef = useRef<HTMLDivElement | null>(null);
  const activeColumnId = detail.snapshot.activePaneId
    ? findPaneColumnId(detail.snapshot, detail.snapshot.activePaneId)
    : null;

  useEffect(() => {
    if (!activeColumnId || !centerScrollRef.current) {
      return;
    }
    if (!detail.snapshot.centerColumnIds.includes(activeColumnId)) {
      return;
    }

    const scrollContainer = centerScrollRef.current;
    const columnElement = scrollContainer.querySelector<HTMLElement>(
      `[data-column-id="${activeColumnId}"]`,
    );
    if (!columnElement) {
      return;
    }

    requestAnimationFrame(() => {
      const target =
        columnElement.offsetLeft - (scrollContainer.clientWidth - columnElement.clientWidth) / 2;
      const maxScroll = Math.max(0, scrollContainer.scrollWidth - scrollContainer.clientWidth);
      scrollContainer.scrollTo({
        left: Math.max(0, Math.min(target, maxScroll)),
        behavior: "smooth",
      });
    });
  }, [activeColumnId, detail.snapshot.centerColumnIds]);

  useEffect(() => {
    if (!isVisible) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      refitConnectedTerminalRuntimes();
    });

    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [detail.snapshot, isVisible]);

  useEffect(() => {
    if (!isVisible || !keyboardNavigationRequest) {
      return;
    }
    if (keyboardNavigationRequest.workspaceId !== detail.workspace.id) {
      return;
    }

    const paneId = keyboardNavigationRequest.paneId ?? detail.snapshot.activePaneId;
    if (!paneId) {
      return;
    }

    const pane = detail.snapshot.panes[paneId];
    if (!pane) {
      return;
    }

    let cancelled = false;
    let attemptsLeft = 8;
    let frame = 0;

    const focusAttempt = () => {
      frame = 0;
      if (cancelled) {
        return;
      }

      const paneElement = document.querySelector<HTMLElement>(`[data-pane-id="${paneId}"]`);
      if (paneElement && focusPaneTarget(paneId, paneElement, pane)) {
        return;
      }

      attemptsLeft -= 1;
      if (attemptsLeft <= 0) {
        return;
      }
      frame = window.requestAnimationFrame(focusAttempt);
    };

    frame = window.requestAnimationFrame(focusAttempt);
    return () => {
      cancelled = true;
      if (frame) {
        window.cancelAnimationFrame(frame);
      }
    };
  }, [detail.snapshot.activePaneId, detail.snapshot.panes, detail.workspace.id, isVisible, keyboardNavigationRequest]);

  const closePane = useCallback(
    (pane: PaneState) => {
      if (pane.type === "shell") {
        const payload = pane.payload as TerminalPanePayload;
        if (payload.sessionId) {
          onCloseSession(payload.sessionId);
        }
        destroyTerminalRuntime(pane.id);
      }
      mutateSnapshot((snapshot) => removePane(snapshot, pane.id));
    },
    [mutateSnapshot, onCloseSession],
  );

  const startColumnResize = useCallback(
    (event: React.PointerEvent<HTMLDivElement>, columnId: string, startWidth: number) => {
      event.preventDefault();
      event.stopPropagation();
      const pointerStartX = event.clientX;

      const handleMove = (moveEvent: PointerEvent) => {
        const delta = moveEvent.clientX - pointerStartX;
        mutateSnapshot((snapshot) => setColumnWidth(snapshot, columnId, startWidth + delta));
      };

      const handleUp = () => {
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
    },
    [mutateSnapshot],
  );

  const startStackResize = useCallback(
    (
      event: React.PointerEvent<HTMLDivElement>,
      columnId: string,
      paneIndex: number,
      column: WorkspaceColumn,
    ) => {
      event.preventDefault();
      event.stopPropagation();
      const stackElement = event.currentTarget.parentElement as HTMLDivElement | null;
      const stackHeight = stackElement?.getBoundingClientRect().height ?? 0;
      if (stackHeight <= 0) {
        return;
      }

      const startFractions = [...column.heightFractions];
      const startY = event.clientY;
      const total = startFractions[paneIndex]! + startFractions[paneIndex + 1]!;
      const minimumFraction = Math.min(0.45, MIN_STACK_PANE_HEIGHT_PX / stackHeight);

      const handleMove = (moveEvent: PointerEvent) => {
        const deltaFraction = (moveEvent.clientY - startY) / stackHeight;
        const nextFractions = [...startFractions];
        const nextLeading = Math.min(
          total - minimumFraction,
          Math.max(minimumFraction, startFractions[paneIndex]! + deltaFraction),
        );
        nextFractions[paneIndex] = nextLeading;
        nextFractions[paneIndex + 1] = total - nextLeading;
        mutateSnapshot((snapshot) => setColumnHeightFractions(snapshot, columnId, nextFractions));
      };

      const handleUp = () => {
        window.removeEventListener("pointermove", handleMove);
        window.removeEventListener("pointerup", handleUp);
      };

      window.addEventListener("pointermove", handleMove);
      window.addEventListener("pointerup", handleUp);
    },
    [mutateSnapshot],
  );

  const handleCenterDrop = useCallback(
    (event: React.DragEvent<HTMLElement>, targetIndex: number) => {
      const payload = getLayoutDragPayload(event);
      if (!payload) {
        return;
      }
      event.preventDefault();

      mutateSnapshot((snapshot) => {
        if (payload.type === "column") {
          return moveColumn(snapshot, payload.columnId, targetIndex);
        }
        return movePaneToNewColumn(snapshot, payload.paneId, targetIndex, currentViewportWidth());
      });
    },
    [mutateSnapshot],
  );

  const handlePinDrop = useCallback(
    (event: React.DragEvent<HTMLElement>, target: "left" | "right") => {
      const payload = getLayoutDragPayload(event);
      if (!payload) {
        return;
      }
      event.preventDefault();

      mutateSnapshot((snapshot) => {
        if (payload.type === "column") {
          return pinColumn(snapshot, payload.columnId, target);
        }
        const moved = movePaneToNewColumn(
          snapshot,
          payload.paneId,
          target === "left" ? 0 : snapshot.centerColumnIds.length,
          currentViewportWidth(),
        );
        const columnId = findPaneColumnId(moved, payload.paneId);
        return columnId ? pinColumn(moved, columnId, target) : moved;
      });
    },
    [mutateSnapshot],
  );

  const handleColumnStackDrop = useCallback(
    (event: React.DragEvent<HTMLElement>, targetColumnId: string) => {
      const payload = getLayoutDragPayload(event);
      if (!payload) {
        return;
      }
      event.preventDefault();

      mutateSnapshot((snapshot) => {
        if (payload.type === "pane") {
          return movePaneToColumn(snapshot, payload.paneId, targetColumnId);
        }
        const sourceColumn = snapshot.columns[payload.columnId];
        if (!sourceColumn || sourceColumn.paneIds.length !== 1) {
          return snapshot;
        }
        return movePaneToColumn(snapshot, sourceColumn.paneIds[0]!, targetColumnId);
      });
    },
    [mutateSnapshot],
  );

  const renderPane = useCallback(
    (pane: PaneState, column: WorkspaceColumn): React.ReactNode => {
      const isActive = detail.snapshot.activePaneId === pane.id;
      const canMoveToNewColumn = column.paneIds.length > 1;

      return (
        <PaneFrame
          key={`${detail.workspace.id}:${pane.id}`}
          pane={pane}
          column={column}
          workspace={detail.workspace}
          isVisible={isVisible}
          notes={detail.notes}
          diffText={detail.workspace.diffText}
          isActive={isActive}
          canMoveToNewColumn={canMoveToNewColumn}
          onFocus={() => activatePane(pane.id)}
          onFixLeft={() => mutateSnapshot((snapshot) => pinColumn(snapshot, column.id, "left"))}
          onFixRight={() => mutateSnapshot((snapshot) => pinColumn(snapshot, column.id, "right"))}
          onMoveToCenter={() => mutateSnapshot((snapshot) => moveColumnToCenter(snapshot, column.id))}
          onMoveToNewColumn={() =>
            mutateSnapshot((snapshot) =>
              movePaneToNewColumn(snapshot, pane.id, undefined, currentViewportWidth()))
          }
          onClose={() => closePane(pane)}
          onDragStartPane={(event) => setLayoutDragPayload(event, { type: "pane", paneId: pane.id })}
          onUpdatePane={(updater) => patchPane(pane.id, updater)}
          subscribeSession={subscribeSession}
          onCreateSession={onCreateSession}
          onFetchSession={onFetchSession}
          onSendSessionInput={onSendSessionInput}
          onResizeSession={onResizeSession}
          onSetSessionFocus={onSetSessionFocus}
          onCreateNote={onCreateNote}
          onSaveNote={onSaveNote}
          onMarkNoteRead={onMarkNoteRead}
        />
      );
    },
    [
      activatePane,
      closePane,
      detail.notes,
      detail.snapshot.activePaneId,
      detail.workspace,
      isVisible,
      mutateSnapshot,
      onCreateNote,
      onCreateSession,
      onFetchSession,
      onSetSessionFocus,
      onMarkNoteRead,
      onResizeSession,
      onSaveNote,
      onSendSessionInput,
      patchPane,
      subscribeSession,
    ],
  );

  const renderColumn = useCallback(
    (columnId: string): React.ReactNode => {
      const column = detail.snapshot.columns[columnId];
      if (!column) {
        return null;
      }

      const isActiveColumn =
        detail.snapshot.activePaneId !== null &&
        column.paneIds.includes(detail.snapshot.activePaneId);

      return (
        <div className="task-column-wrap" key={column.id}>
          <div
            className={`task-column ${isActiveColumn ? "active" : ""} ${column.pinned ? `pinned-${column.pinned}` : ""}`}
            data-column-id={column.id}
            style={{ width: `${column.widthPx}px` }}
            onDragOver={(event) => {
              if (getLayoutDragPayload(event)) {
                event.preventDefault();
              }
            }}
            onDrop={(event) => handleColumnStackDrop(event, column.id)}
          >
            <div className="task-column-stack">
              {column.paneIds.map((paneId, index) => {
                const pane = detail.snapshot.panes[paneId];
                if (!pane) {
                  return null;
                }

                const fraction = column.heightFractions[index] ?? 1 / column.paneIds.length;
                return (
                  <React.Fragment key={paneId}>
                    <div className="task-pane-slot" style={{ flex: `${fraction} 1 0` }}>
                      {renderPane(pane, column)}
                    </div>
                    {index < column.paneIds.length - 1 && (
                      <div
                        className="stack-resizer"
                        onPointerDown={(event) => startStackResize(event, column.id, index, column)}
                      />
                    )}
                  </React.Fragment>
                );
              })}
            </div>
          </div>
          <div
            className="column-resizer"
            onPointerDown={(event) => startColumnResize(event, column.id, column.widthPx)}
          />
        </div>
      );
    },
    [
      detail.snapshot.activePaneId,
      detail.snapshot.columns,
      detail.snapshot.panes,
      handleColumnStackDrop,
      renderPane,
      startColumnResize,
      startStackResize,
    ],
  );

  if (Object.keys(detail.snapshot.columns).length === 0) {
    return (
      <div className="taskspace">
        <div className="empty-stage">
          <p>No panes are open.</p>
        </div>
      </div>
    );
  }

  return (
    <div className="taskspace">
      <aside
        className={`pinned-slot left ${detail.snapshot.pinnedLeftColumnId ? "" : "empty"}`}
        onDragOver={(event) => {
          if (getLayoutDragPayload(event)) {
            event.preventDefault();
          }
        }}
        onDrop={(event) => handlePinDrop(event, "left")}
      >
        {detail.snapshot.pinnedLeftColumnId
          ? renderColumn(detail.snapshot.pinnedLeftColumnId)
          : <div className="pin-placeholder">Fix left</div>}
      </aside>

      <section className="center-taskspace">
        <div className="center-column-scroll" ref={centerScrollRef}>
          <div className="center-column-strip">
            {detail.snapshot.centerColumnIds.length === 0 && (
              <div className="empty-stage compact">
                <p>Add a pane to start a column.</p>
              </div>
            )}

            {detail.snapshot.centerColumnIds.map((columnId, index) => (
              <React.Fragment key={`drop-${columnId}`}>
                <div
                  className="center-drop-zone"
                  onDragOver={(event) => {
                    if (getLayoutDragPayload(event)) {
                      event.preventDefault();
                    }
                  }}
                  onDrop={(event) => handleCenterDrop(event, index)}
                />
                {renderColumn(columnId)}
              </React.Fragment>
            ))}

            <div
              className="center-drop-zone tail"
              onDragOver={(event) => {
                if (getLayoutDragPayload(event)) {
                  event.preventDefault();
                }
              }}
              onDrop={(event) => handleCenterDrop(event, detail.snapshot.centerColumnIds.length)}
            />
          </div>
        </div>
      </section>

      <aside
        className={`pinned-slot right ${detail.snapshot.pinnedRightColumnId ? "" : "empty"}`}
        onDragOver={(event) => {
          if (getLayoutDragPayload(event)) {
            event.preventDefault();
          }
        }}
        onDrop={(event) => handlePinDrop(event, "right")}
      >
        {detail.snapshot.pinnedRightColumnId
          ? renderColumn(detail.snapshot.pinnedRightColumnId)
          : <div className="pin-placeholder">Fix right</div>}
      </aside>
    </div>
  );
}

type PaneFrameProps = {
  pane: PaneState;
  column: WorkspaceColumn;
  workspace: WorkspaceSummary;
  isVisible: boolean;
  notes: NoteRecord[];
  diffText: string;
  isActive: boolean;
  canMoveToNewColumn: boolean;
  onFocus: () => void;
  onFixLeft: () => void;
  onFixRight: () => void;
  onMoveToCenter: () => void;
  onMoveToNewColumn: () => void;
  onClose: () => void;
  onDragStartPane: (event: React.DragEvent<HTMLElement>) => void;
  onUpdatePane: (updater: (pane: PaneState) => PaneState) => void;
  subscribeSession: WorkspaceTaskspaceProps["subscribeSession"];
  onCreateSession: WorkspaceTaskspaceProps["onCreateSession"];
  onFetchSession: WorkspaceTaskspaceProps["onFetchSession"];
  onSendSessionInput: WorkspaceTaskspaceProps["onSendSessionInput"];
  onResizeSession: WorkspaceTaskspaceProps["onResizeSession"];
  onSetSessionFocus: WorkspaceTaskspaceProps["onSetSessionFocus"];
  onCreateNote: WorkspaceTaskspaceProps["onCreateNote"];
  onSaveNote: WorkspaceTaskspaceProps["onSaveNote"];
  onMarkNoteRead: WorkspaceTaskspaceProps["onMarkNoteRead"];
};

function PaneFrame(props: PaneFrameProps): React.ReactElement {
  const {
    pane,
    column,
    workspace,
    isVisible,
    notes,
    diffText,
    isActive,
    canMoveToNewColumn,
    onFocus,
    onFixLeft,
    onFixRight,
    onMoveToCenter,
    onMoveToNewColumn,
    onClose,
    onDragStartPane,
    onUpdatePane,
    subscribeSession,
    onCreateSession,
    onFetchSession,
    onSendSessionInput,
    onResizeSession,
    onSetSessionFocus,
    onCreateNote,
    onSaveNote,
    onMarkNoteRead,
  } = props;
  const [copyState, setCopyState] = useState<"idle" | "copied" | "error">("idle");
  const terminalPayload =
    pane.type === "shell" ? pane.payload as TerminalPanePayload : null;
  const attentionClassName =
    terminalPayload && supportsTerminalAttention(terminalPayload.kind)
      ? agentAttentionClassName(terminalPayload.agentAttentionState)
      : null;
  const attentionLabel =
    terminalPayload && supportsTerminalAttention(terminalPayload.kind)
      ? agentAttentionLabel(terminalPayload.agentAttentionState)
      : null;
  const resumeCommand =
    terminalPayload?.embeddedSession
      ? terminalPayload.command
      : null;

  useEffect(() => {
    if (copyState === "idle") {
      return;
    }

    const timer = window.setTimeout(() => {
      setCopyState("idle");
    }, 1_500);

    return () => {
      window.clearTimeout(timer);
    };
  }, [copyState]);

  return (
    <section
      className={`pane-frame ${isActive ? "active" : ""}`}
      data-pane-id={pane.id}
      data-pane-type={pane.type}
      tabIndex={-1}
      onMouseDown={onFocus}
    >
      <header className="pane-header">
        <div className="pane-title-row">
          <button className="drag-handle" draggable onDragStart={onDragStartPane}>
            ::
          </button>
          {attentionClassName && (
            <span
              className={`agent-attention-dot ${attentionClassName}`}
              title={attentionLabel ?? undefined}
            />
          )}
          <div className="pane-title">{pane.title}</div>
        </div>
        <div className="pane-actions">
          {resumeCommand && (
            <button
              className={`copy-resume-button ${copyState === "error" ? "copy-error" : ""}`}
              title={resumeCommand}
              onClick={() => {
                void copyTextToClipboard(resumeCommand)
                  .then(() => setCopyState("copied"))
                  .catch(() => setCopyState("error"));
              }}
            >
              {copyState === "copied" ? "Copied" : copyState === "error" ? "Copy failed" : "Copy resume"}
            </button>
          )}
          {column.pinned === null ? (
            <>
              <button onClick={onFixLeft}>Fix left</button>
              <button onClick={onFixRight}>Fix right</button>
            </>
          ) : (
            <button onClick={onMoveToCenter}>Center</button>
          )}
          <button onClick={onMoveToNewColumn} disabled={!canMoveToNewColumn}>
            New column
          </button>
          <button className="danger-subtle" onClick={onClose}>
            Close
          </button>
        </div>
      </header>
      <div className="pane-body">
        {pane.type === "diff" && <DiffPane diffText={diffText} />}
        {pane.type === "shell" && (
          <TerminalPane
            pane={pane}
            workspace={workspace}
            isVisible={isVisible}
            isActive={isActive}
            subscribeSession={subscribeSession}
            onCreateSession={onCreateSession}
            onFetchSession={onFetchSession}
            onSendSessionInput={onSendSessionInput}
            onResizeSession={onResizeSession}
            onSetSessionFocus={onSetSessionFocus}
            onUpdatePane={onUpdatePane}
          />
        )}
        {pane.type === "note" && (
          <NotePane
            pane={pane}
            notes={notes}
            onUpdatePane={onUpdatePane}
            onCreateNote={onCreateNote}
            onSaveNote={onSaveNote}
            onMarkNoteRead={onMarkNoteRead}
          />
        )}
        {pane.type === "browser" && (
          <BrowserPane
            pane={pane}
            isActive={isActive}
            isVisible={isVisible}
            onUpdatePane={onUpdatePane}
          />
        )}
      </div>
    </section>
  );
}

function DiffPane({ diffText }: { diffText: string }): React.ReactElement {
  return (
    <pre className="diff-pane">
      {diffText.trim() || "No working-copy changes."}
    </pre>
  );
}

function scrubTerminalSurface(container: HTMLDivElement | null): void {
  if (!container) {
    return;
  }

  for (const node of Array.from(container.childNodes)) {
    if (node instanceof HTMLCanvasElement || node instanceof HTMLTextAreaElement) {
      continue;
    }
    container.removeChild(node);
  }
}

function resetTerminalSurface(container: HTMLDivElement | null): void {
  if (!container) {
    return;
  }
  container.replaceChildren();
}

function TerminalPane({
  pane,
  workspace,
  isVisible,
  isActive,
  subscribeSession,
  onCreateSession,
  onFetchSession,
  onSendSessionInput,
  onResizeSession,
  onSetSessionFocus,
  onUpdatePane,
}: {
  pane: PaneState;
  workspace: WorkspaceSummary;
  isVisible: boolean;
  isActive: boolean;
  subscribeSession: WorkspaceTaskspaceProps["subscribeSession"];
  onCreateSession: WorkspaceTaskspaceProps["onCreateSession"];
  onFetchSession: WorkspaceTaskspaceProps["onFetchSession"];
  onSendSessionInput: WorkspaceTaskspaceProps["onSendSessionInput"];
  onResizeSession: WorkspaceTaskspaceProps["onResizeSession"];
  onSetSessionFocus: WorkspaceTaskspaceProps["onSetSessionFocus"];
  onUpdatePane: (updater: (pane: PaneState) => PaneState) => void;
}): React.ReactElement {
  const payload = pane.payload as TerminalPanePayload;
  const containerRef = useRef<HTMLDivElement | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const runtimeRef = useRef<TerminalRuntime | null>(null);
  const payloadRef = useRef(payload);
  const subscribeSessionRef = useRef(subscribeSession);
  const onCreateSessionRef = useRef(onCreateSession);
  const onFetchSessionRef = useRef(onFetchSession);
  const onSendSessionInputRef = useRef(onSendSessionInput);
  const onResizeSessionRef = useRef(onResizeSession);
  const onSetSessionFocusRef = useRef(onSetSessionFocus);
  const onUpdatePaneRef = useRef(onUpdatePane);
  const pendingSessionStartRef = useRef<Promise<void> | null>(null);
  const resizeFrameRef = useRef<number | null>(null);
  const [statusText, setStatusText] = useState<string>(terminalStatusLabel(payload));

  useEffect(() => {
    payloadRef.current = payload;
    setStatusText(terminalStatusLabel(payload));
  }, [payload.autoStart, payload.exitCode, payload.sessionId, payload.sessionState]);

  useEffect(() => {
    subscribeSessionRef.current = subscribeSession;
    onCreateSessionRef.current = onCreateSession;
    onFetchSessionRef.current = onFetchSession;
    onSendSessionInputRef.current = onSendSessionInput;
    onResizeSessionRef.current = onResizeSession;
    onSetSessionFocusRef.current = onSetSessionFocus;
    onUpdatePaneRef.current = onUpdatePane;
  }, [
    onCreateSession,
    onFetchSession,
    onResizeSession,
    onSetSessionFocus,
    onSendSessionInput,
    onUpdatePane,
    subscribeSession,
  ]);

  useEffect(() => {
    if (!payload.sessionId || !supportsTerminalAttention(payload.kind)) {
      return;
    }
    const focused = isVisible && isActive;
    onSetSessionFocusRef.current(payload.sessionId, focused);
    return () => {
      onSetSessionFocusRef.current(payload.sessionId!, false);
    };
  }, [isActive, isVisible, payload.kind, payload.sessionId]);

  const resetSurface = useCallback(() => {
    resetTerminalSurface(containerRef.current);
  }, []);

  const focusTerminalInput = useCallback((): boolean => {
    const container = containerRef.current;
    if (!container) {
      return false;
    }
    const focusTarget = container.querySelector(
      "textarea, [contenteditable='true'], .terminal-runtime-host",
    );
    if (focusTarget instanceof HTMLElement) {
      focusTarget.focus();
      logTerminalUi("focus-terminal", {
        paneId: pane.id,
        target: describeElement(focusTarget),
        activeElement: describeElement(document.activeElement),
      });
      return document.activeElement === focusTarget;
    }
    container.focus();
    logTerminalUi("focus-terminal", {
      paneId: pane.id,
      target: "container",
      activeElement: describeElement(document.activeElement),
    });
    return document.activeElement === container;
  }, [pane.id]);

  useEffect(() => {
    paneFocusRegistry.set(pane.id, focusTerminalInput);
    return () => {
      if (paneFocusRegistry.get(pane.id) === focusTerminalInput) {
        paneFocusRegistry.delete(pane.id);
      }
    };
  }, [focusTerminalInput, pane.id]);

  const currentTerminalSize = useCallback(() => {
    const active = terminalRef.current as unknown as { cols?: number; rows?: number } | null;
    return {
      cols: Math.max(20, active?.cols ?? 120),
      rows: Math.max(5, active?.rows ?? 32),
    };
  }, []);

  const scheduleFitAndResizeSync = useCallback(() => {
    if (resizeFrameRef.current !== null) {
      window.cancelAnimationFrame(resizeFrameRef.current);
    }

    resizeFrameRef.current = window.requestAnimationFrame(() => {
      resizeFrameRef.current = null;
      const container = containerRef.current;
      const runtime = runtimeRef.current;
      if (!container || !runtime || !runtime.host.isConnected) {
        return;
      }

      runtime.fitAddon.fit();

      const activePayload = payloadRef.current;
      if (activePayload.sessionState !== "live" || !activePayload.sessionId) {
        return;
      }

      const active = runtime.term as unknown as { cols?: number; rows?: number };
      const cols = Math.max(20, active.cols ?? 120);
      const rows = Math.max(5, active.rows ?? 32);
      onResizeSessionRef.current(activePayload.sessionId, cols, rows);
    });
  }, [pane.id]);

  useEffect(() => {
    if (!isVisible) {
      return;
    }

    const term = terminalRef.current;
    if (!term) {
      return;
    }

    scheduleFitAndResizeSync();
  }, [isVisible, scheduleFitAndResizeSync]);

  useEffect(() => {
    if (!isVisible) {
      return;
    }

    const container = containerRef.current;
    if (!container || typeof ResizeObserver === "undefined") {
      return;
    }

    const observer = new ResizeObserver(() => {
      scheduleFitAndResizeSync();
    });
    observer.observe(container);
    scheduleFitAndResizeSync();

    return () => {
      observer.disconnect();
    };
  }, [isVisible, pane.id, scheduleFitAndResizeSync]);

  const startSession = useCallback(
    (kind: TerminalKind, options?: { focus?: boolean }) => {
      const pending = pendingSessionStartRef.current;
      if (pending) {
        return pending;
      }

      onUpdatePaneRef.current((current) => ({
        ...current,
        payload: {
          ...(current.payload as TerminalPanePayload),
          sessionId: null,
          autoStart: false,
          exitCode: null,
        },
      }));

      const { cols, rows } = currentTerminalSize();
      const next = onCreateSessionRef
        .current(workspace.id, pane.id, kind, cols, rows)
        .then((session) => {
          onResizeSessionRef.current(session.id, cols, rows);
          setStatusText("live");
          if (options?.focus) {
            focusTerminalInput();
          }
        })
        .catch((error) => {
          const message = error instanceof Error ? error.message : String(error);
          logTerminalUi("session-start-error", {
            paneId: pane.id,
            message,
          });
          setStatusText("inactive");
          onUpdatePaneRef.current((current) => ({
            ...current,
            payload: {
              ...(current.payload as TerminalPanePayload),
              sessionId: null,
              sessionState: "missing",
              autoStart: false,
            },
          }));
        })
        .finally(() => {
          pendingSessionStartRef.current = null;
        });
      pendingSessionStartRef.current = next;
      return next;
    },
    [currentTerminalSize, focusTerminalInput, pane.id, workspace.id],
  );

  useEffect(() => {
    let disposed = false;
    let runtime: TerminalRuntime | null = null;
    const mount = async () => {
      await ensureGhostty();
      if (!containerRef.current || disposed) {
        return;
      }

      runtime = getOrCreateTerminalRuntime(pane.id);
      runtimeRef.current = runtime;
      runtime.sendInput = (sessionId, data) => onSendSessionInputRef.current(sessionId, data);
      runtime.resizeSession = (sessionId, cols, rows) =>
        onResizeSessionRef.current(sessionId, cols, rows);
      runtime.onExit = (exitCode) => {
        setStatusText(`exit ${exitCode ?? 0}`);
        onUpdatePaneRef.current((current) => ({
          ...current,
          payload: {
            ...(current.payload as TerminalPanePayload),
            sessionId: null,
            sessionState: "stopped",
            exitCode,
            autoStart: false,
          },
        }));
      };
      attachTerminalRuntime(runtime, containerRef.current);
      terminalRef.current = runtime.term;

      if (payload.sessionId) {
        if (runtime.sessionId === payload.sessionId) {
          setStatusText(payload.sessionState === "live" ? "live" : terminalStatusLabel(payload));
          if (isVisible) {
            const { cols, rows } = currentTerminalSize();
            onResizeSessionRef.current(payload.sessionId, cols, rows);
          }
          return;
        }

        try {
          const session = await withTimeout(
            onFetchSessionRef.current(payload.sessionId),
            TERMINAL_REQUEST_TIMEOUT_MS,
            "Terminal session fetch",
          );
          if (disposed || !runtime) {
            return;
          }
          setStatusText(session.state === "live" ? "live" : `exit ${session.exitCode ?? 0}`);

          if (session.state === "live") {
            if (isVisible) {
              const { cols, rows } = currentTerminalSize();
              onResizeSessionRef.current(session.id, cols, rows);
            }
            bindTerminalRuntimeSession(
              runtime,
              payload.sessionId,
              subscribeSessionRef.current,
            );
            resetTerminalRuntime(runtime);
            if (session.screen) {
              runtime.enqueueWrite(session.screen);
            }
          } else {
            onUpdatePaneRef.current((current) => ({
              ...current,
              payload: {
                ...(current.payload as TerminalPanePayload),
                sessionId: null,
                sessionState: session.state,
                exitCode: session.exitCode,
                autoStart: false,
              },
            }));
          }
        } catch {
          if (!disposed) {
            const shouldRestart = payload.sessionState === "live" || payload.autoStart;
            setStatusText(shouldRestart ? "starting" : "inactive");
            onUpdatePaneRef.current((current) => ({
              ...current,
              payload: {
                ...(current.payload as TerminalPanePayload),
                sessionId: null,
                sessionState: "missing",
                autoStart: shouldRestart,
                exitCode: shouldRestart ? null : (current.payload as TerminalPanePayload).exitCode,
              },
            }));
          }
        }
      } else {
        if (payload.autoStart) {
          setStatusText("starting");
          await new Promise<void>((resolve) => {
            requestAnimationFrame(() => {
              if (runtime?.host.isConnected) {
                runtime.fitAddon.fit();
              }
              resolve();
            });
          });
          if (disposed) {
            return;
          }
          await startSession(payload.kind, { focus: false });
        } else {
          setStatusText(terminalStatusLabel(payload));
        }
      }
    };

    void mount()
      .catch(() => {
        if (!disposed) {
          setStatusText("inactive");
        }
      });

    return () => {
      disposed = true;
      if (resizeFrameRef.current !== null) {
        window.cancelAnimationFrame(resizeFrameRef.current);
        resizeFrameRef.current = null;
      }
      if (runtime) {
        runtime.onExit = () => {};
        runtime.flushInput();
        detachTerminalRuntime(runtime);
      }
      runtimeRef.current = null;
      if (terminalRef.current === runtime?.term) {
        terminalRef.current = null;
      }
      resetSurface();
    };
  }, [
    focusTerminalInput,
    pane.id,
    payload.autoStart,
    payload.kind,
    payload.sessionId,
    payload.sessionState,
    resetSurface,
    startSession,
    workspace.id,
  ]);

  return (
    <div className="terminal-pane">
      <div
        className="terminal-surface"
        ref={containerRef}
        onClick={() => {
          focusTerminalInput();
        }}
      />
      {payload.sessionState !== "live" && !payload.autoStart && (
        <button
          className="restart-button"
          onClick={async () => {
            setStatusText("starting");
            await startSession(payload.kind, { focus: true });
          }}
        >
          Restart session
        </button>
      )}
    </div>
  );
}

function NotePane({
  pane,
  notes,
  onUpdatePane,
  onCreateNote,
  onSaveNote,
  onMarkNoteRead,
}: {
  pane: PaneState;
  notes: NoteRecord[];
  onUpdatePane: (updater: (pane: PaneState) => PaneState) => void;
  onCreateNote: WorkspaceTaskspaceProps["onCreateNote"];
  onSaveNote: WorkspaceTaskspaceProps["onSaveNote"];
  onMarkNoteRead: WorkspaceTaskspaceProps["onMarkNoteRead"];
}): React.ReactElement {
  const payload = pane.payload as NotePanePayload;
  const selected = notes.find((note) => note.path === payload.notePath) ?? notes[0] ?? null;
  const [draft, setDraft] = useState(selected?.body ?? "");

  useEffect(() => {
    setDraft(selected?.body ?? "");
    if (selected) {
      onMarkNoteRead(selected.path).catch(() => {
        return;
      });
    }
  }, [onMarkNoteRead, selected?.path, selected?.updatedAt]);

  useEffect(() => {
    if (!selected) {
      return;
    }

    const timer = window.setTimeout(() => {
      if (draft !== selected.body) {
        void onSaveNote(selected.path, draft);
      }
    }, 500);

    return () => {
      window.clearTimeout(timer);
    };
  }, [draft, onSaveNote, selected]);

  return (
    <div className="note-pane">
      <aside className="note-list">
        <button
          className="full-width"
          onClick={async () => {
            const note = await onCreateNote("note");
            onUpdatePane((current) => ({
              ...current,
              payload: {
                ...(current.payload as NotePanePayload),
                notePath: note.path,
              },
            }));
          }}
        >
          New note
        </button>
        {notes.map((note) => (
          <button
            key={note.path}
            className={`note-item ${selected?.path === note.path ? "selected" : ""}`}
            onClick={() =>
              onUpdatePane((current) => ({
                ...current,
                payload: {
                  ...(current.payload as NotePanePayload),
                  notePath: note.path,
                },
              }))
            }
          >
            <span>{note.title}</span>
            {note.unread && <span className="status-dot draft" />}
          </button>
        ))}
      </aside>
      <section className="note-editor">
        {selected ? (
          <>
            <div className="pane-subheader">
              <span>{selected.fileName}</span>
              <span>{selected.unread ? "unread" : "read"}</span>
            </div>
            <textarea value={draft} onChange={(event) => setDraft(event.target.value)} />
          </>
        ) : (
          <div className="empty-stage">
            <p>Create a note to start writing.</p>
          </div>
        )}
      </section>
    </div>
  );
}

function BrowserPane({
  pane,
  isActive,
  isVisible,
  onUpdatePane,
}: {
  pane: PaneState;
  isActive: boolean;
  isVisible: boolean;
  onUpdatePane: (updater: (pane: PaneState) => PaneState) => void;
}): React.ReactElement {
  const payload = pane.payload as BrowserPanePayload;
  const webviewRef = useRef<HTMLElement | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const toolbarId = `browser-toolbar-${pane.id}`;
  const [urlValue, setUrlValue] = useState(payload.url);

  useEffect(() => {
    setUrlValue(payload.url);
  }, [payload.url]);

  useEffect(() => {
    const webview = webviewRef.current as HTMLElement & {
      webviewId?: number | null;
      toggleHidden?: (value?: boolean) => void;
      togglePassthrough?: (value?: boolean) => void;
    } | null;
    if (!webview) {
      return;
    }

    const timers: number[] = [];
    let disposed = false;

    const syncInteractionState = () => {
      if (disposed) {
        return;
      }
      webview.toggleHidden?.(!isVisible);
      webview.togglePassthrough?.(!isActive);
    };

    syncInteractionState();

    if (webview.webviewId == null) {
      for (const delayMs of [16, 64, 160, 320]) {
        timers.push(window.setTimeout(syncInteractionState, delayMs));
      }
    }

    return () => {
      disposed = true;
      for (const timer of timers) {
        window.clearTimeout(timer);
      }
    };
  }, [isActive, isVisible]);

  useEffect(() => {
    const webview = webviewRef.current as HTMLElement & {
      on: (name: string, listener: (event: CustomEvent) => void) => void;
      off: (name: string, listener: (event: CustomEvent) => void) => void;
      src: string | null;
      goBack: () => void;
      goForward: () => void;
      reload: () => void;
    } | null;

    if (!webview) {
      return;
    }

    const listener = (event: CustomEvent) => {
      const detail = event.detail as { url?: string } | string;
      const url = typeof detail === "string" ? detail : detail?.url;
      if (!url) {
        return;
      }
      setUrlValue(url);
      onUpdatePane((current) => ({
        ...current,
        payload: {
          ...(current.payload as BrowserPanePayload),
          url,
        },
      }));
    };

    webview.on("did-navigate", listener);
    webview.on("did-navigate-in-page", listener);

    return () => {
      webview.off("did-navigate", listener);
      webview.off("did-navigate-in-page", listener);
    };
  }, [onUpdatePane]);

  return (
    <div className="browser-pane">
      <div className="browser-toolbar browser-toolbar-compact" id={toolbarId}>
        <button
          className="browser-nav-button"
          title="Back"
          aria-label="Back"
          onClick={() => (webviewRef.current as any)?.goBack?.()}
        >
          ←
        </button>
        <button
          className="browser-nav-button"
          title="Forward"
          aria-label="Forward"
          onClick={() => (webviewRef.current as any)?.goForward?.()}
        >
          →
        </button>
        <button
          className="browser-nav-button"
          title="Reload"
          aria-label="Reload"
          onClick={() => (webviewRef.current as any)?.reload?.()}
        >
          ↻
        </button>
        <form
          className="browser-form browser-omnibox"
          onSubmit={(event) => {
            event.preventDefault();
            const normalized = urlValue.match(/^https?:\/\//) ? urlValue : `https://${urlValue}`;
            (webviewRef.current as any).src = normalized;
            onUpdatePane((current) => ({
              ...current,
              payload: {
                ...(current.payload as BrowserPanePayload),
                url: normalized,
              },
            }));
          }}
        >
          <span className="browser-omnibox-prefix" aria-hidden="true">
            //
          </span>
          <input
            ref={inputRef}
            className="browser-omnibox-input"
            value={urlValue}
            placeholder="Enter URL"
            onFocus={(event) => event.currentTarget.select()}
            onChange={(event) => setUrlValue(event.target.value)}
          />
        </form>
      </div>
      <electrobun-webview
        {...({ masks: `#${toolbarId}` } as any)}
        id={`browser-${pane.id}`}
        ref={webviewRef as any}
        className="embedded-webview"
        src={payload.url}
        renderer="native"
      />
    </div>
  );
}

createRoot(document.getElementById("root")!).render(<App />);

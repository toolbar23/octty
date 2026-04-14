import {
  init as initGhostty,
  Terminal,
  FitAddon,
  type ITheme,
} from "ghostty-web";
import "./index.css";
import React, {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createRoot } from "react-dom/client";
import {
  addPane,
  findPaneColumnId,
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
  BrowserDownloadState,
  BrowserEventEnvelope,
  BrowserFindResult,
  BrowserPanePayload,
  BrowserViewBounds,
  BrowserViewState,
  NoteRecord,
  NotePanePayload,
  PaneState,
  PaneType,
  SessionSnapshot,
  TerminalKind,
  TerminalPanePayload,
  WorkspaceColumn,
  WorkspaceDetail,
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
import {
  appShortcutActionForKeyEvent,
  workspaceShortcutIndexForAction,
  workspaceShortcutLabel,
  workspaceShortcutTargets,
} from "../shared/app-shortcuts";
import {
  shouldRemapShiftEnterToCtrlJ,
  terminalClipboardShortcutActionForKeyEvent,
  type TerminalClipboardShortcutAction,
} from "../shared/terminal-shortcuts";
import {
  isAgentTerminalKind,
  shouldCloseTerminalPaneOnExit,
  shouldShowTerminalRestart,
  supportsTerminalAttention,
  terminalRestoreRerenderMode,
} from "../shared/terminal-kind";
import {
  defaultTerminalAppearanceConfig,
  type TerminalAppearanceConfig,
} from "../shared/terminal-font";
import { workspaceStatusBadges } from "../shared/workspace-state";
import {
  clampBrowserZoom,
  normalizeBrowserNavigationInput,
  visibleRectIntersection,
  type BrowserSurfaceRect,
} from "../shared/browser-utils";
import { desktopClient } from "./desktop-client";
const debugTerminal =
  document.querySelector('meta[name="octty-debug-terminal"], meta[name="workspace-orbit-debug-terminal"]')?.getAttribute("content") ===
  "1";
const debugMessageRates =
  document
    .querySelector('meta[name="octty-debug-message-rates"], meta[name="workspace-orbit-debug-message-rates"]')
    ?.getAttribute("content") === "1";
const ghosttyRenderIntervalMs = Math.max(
  16,
  Number.parseInt(
    document
      .querySelector(
        'meta[name="octty-ghostty-render-interval-ms"], meta[name="workspace-orbit-ghostty-render-interval-ms"]',
      )
      ?.getAttribute("content") ?? "100",
    10,
  ) || 100,
);
let ghosttyInitPromise: Promise<void> | null = null;
let forwardTerminalUiDebug:
  | ((message: string, details: Record<string, unknown>) => void)
  | null = null;
const MAX_TERMINAL_WRITE_CHUNK = 16_384;
const MAX_TERMINAL_WRITE_CHUNKS_PER_TICK = 4;
const TERMINAL_INPUT_BATCH_DELAY_MS = 4;
const TERMINAL_INPUT_BATCH_SIZE = 256;
const TERMINAL_REQUEST_TIMEOUT_MS = 8_000;
const FOCUSED_TERMINAL_RENDER_BURST_MS = 160;
const MIN_STACK_PANE_HEIGHT_PX = 96;
const KEYBOARD_COLUMN_RESIZE_STEP_PX = 80;
const HORIZONTAL_WHEEL_FOCUS_THRESHOLD_PX = 80;
const HORIZONTAL_WHEEL_FOCUS_RESET_MS = 240;
const WHEEL_LINE_DELTA_PX = 40;
const CTRL_J = "\x0a";

type DebugRateBucket = {
  count: number;
  sample?: Record<string, unknown>;
};

const rendererDebugRateBuckets = new Map<string, DebugRateBucket>();
let rendererDebugRateTimer: number | null = null;
let ghosttyRenderLoopPatched = false;

type PatchedGhosttyTerminal = Terminal & {
  __octtyRenderLoopMode?: TerminalRenderLoopMode;
  __octtyRenderLoopHandle?: number | null;
  __octtyRenderLoopSchedule?: "raf" | "timeout" | null;
  __octtyRenderLoopBurstUntil?: number;
};

type TerminalRenderLoopMode = "stopped" | "focused" | "visible";

function clearTerminalRenderLoopHandle(term: PatchedGhosttyTerminal): void {
  if (term.__octtyRenderLoopHandle == null) {
    return;
  }
  if (term.__octtyRenderLoopSchedule === "raf") {
    window.cancelAnimationFrame(term.__octtyRenderLoopHandle);
  } else {
    window.clearTimeout(term.__octtyRenderLoopHandle);
  }
  term.__octtyRenderLoopHandle = null;
  term.__octtyRenderLoopSchedule = null;
}

function renderTerminalFrame(term: PatchedGhosttyTerminal): void {
  (term as any).renderer.render(
    (term as any).wasmTerm,
    false,
    (term as any).viewportY,
    term,
    (term as any).scrollbarOpacity,
  );
  const cursor = (term as any).wasmTerm.getCursor();
  if (cursor.y !== (term as any).lastCursorY) {
    (term as any).lastCursorY = cursor.y;
    (term as any).cursorMoveEmitter.fire();
  }
}

function scheduleTerminalRenderLoop(term: PatchedGhosttyTerminal): void {
  clearTerminalRenderLoopHandle(term);
  if (
    term.__octtyRenderLoopMode === "stopped" ||
    (term as any).isDisposed ||
    !(term as any).isOpen
  ) {
    return;
  }

  const burstUntil = term.__octtyRenderLoopBurstUntil ?? 0;
  if (term.__octtyRenderLoopMode === "focused" && window.performance.now() < burstUntil) {
    term.__octtyRenderLoopSchedule = "raf";
    term.__octtyRenderLoopHandle = window.requestAnimationFrame(() => {
      renderTerminalFrame(term);
      scheduleTerminalRenderLoop(term);
    });
    return;
  }

  term.__octtyRenderLoopSchedule = "timeout";
  term.__octtyRenderLoopHandle = window.setTimeout(() => {
    renderTerminalFrame(term);
    scheduleTerminalRenderLoop(term);
  }, ghosttyRenderIntervalMs);
}

function boostTerminalRenderLoop(term: Terminal, durationMs = FOCUSED_TERMINAL_RENDER_BURST_MS): void {
  const patchedTerm = term as PatchedGhosttyTerminal;
  if (patchedTerm.__octtyRenderLoopMode !== "focused") {
    return;
  }
  patchedTerm.__octtyRenderLoopBurstUntil = Math.max(
    patchedTerm.__octtyRenderLoopBurstUntil ?? 0,
    window.performance.now() + durationMs,
  );
  if (patchedTerm.__octtyRenderLoopSchedule === "raf" && patchedTerm.__octtyRenderLoopHandle != null) {
    return;
  }
  scheduleTerminalRenderLoop(patchedTerm);
}

function setTerminalRenderLoopMode(term: Terminal, mode: TerminalRenderLoopMode): void {
  const patchedTerm = term as PatchedGhosttyTerminal;
  if (patchedTerm.__octtyRenderLoopMode === mode && patchedTerm.__octtyRenderLoopHandle != null) {
    return;
  }

  patchedTerm.__octtyRenderLoopMode = mode;
  if (mode === "focused") {
    patchedTerm.__octtyRenderLoopBurstUntil = Math.max(
      patchedTerm.__octtyRenderLoopBurstUntil ?? 0,
      window.performance.now() + FOCUSED_TERMINAL_RENDER_BURST_MS,
    );
  }
  scheduleTerminalRenderLoop(patchedTerm);
}

function setTerminalRenderLoopActive(term: Terminal, active: boolean): void {
  setTerminalRenderLoopMode(term, active ? "visible" : "stopped");
}

function trackRendererDebugRate(key: string, sample?: Record<string, unknown>): void {
  if (!debugMessageRates) {
    return;
  }

  const bucket = rendererDebugRateBuckets.get(key) ?? { count: 0 };
  bucket.count += 1;
  if (sample) {
    bucket.sample = sample;
  }
  rendererDebugRateBuckets.set(key, bucket);

  if (rendererDebugRateTimer !== null) {
    return;
  }

  rendererDebugRateTimer = window.setTimeout(() => {
    rendererDebugRateTimer = null;
    if (rendererDebugRateBuckets.size === 0) {
      return;
    }

    const summary = Array.from(rendererDebugRateBuckets.entries())
      .sort((left, right) => right[1].count - left[1].count)
      .map(([bucketKey, value]) => ({
        key: bucketKey,
        count: value.count,
        sample: value.sample,
      }));
    rendererDebugRateBuckets.clear();
    console.log("[debug-rates][renderer]", summary);
    forwardTerminalUiDebug?.("debug-rates-renderer", { summary });
  }, 1_000);
}

type WorkspaceOrbitWindow = Window & {
  __workspaceOrbitHandleClosePane?: () => void;
  __workspaceOrbitInvokeShortcut?: (action: string) => void;
};

function desktop() {
  return desktopClient.bridge();
}

type ShortcutBridgeEvent = CustomEvent<string>;

const desktopPlatform = window.octtyDesktop?.platform ?? null;

function patchGhosttyRenderLoop(): void {
  if (ghosttyRenderLoopPatched) {
    return;
  }

  const terminalPrototype = Terminal.prototype as unknown as {
    startRenderLoop?: (this: PatchedGhosttyTerminal) => void;
    dispose?: (this: PatchedGhosttyTerminal) => void;
  };
  const originalDispose = terminalPrototype.dispose;

  terminalPrototype.startRenderLoop = function patchedStartRenderLoop(
    this: PatchedGhosttyTerminal,
  ): void {
    this.__octtyRenderLoopMode ??= "visible";
    scheduleTerminalRenderLoop(this);
  };

  terminalPrototype.dispose = function patchedDispose(this: PatchedGhosttyTerminal): void {
    this.__octtyRenderLoopMode = "stopped";
    clearTerminalRenderLoopHandle(this);
    originalDispose?.call(this);
  };

  ghosttyRenderLoopPatched = true;
}

async function ensureGhostty(): Promise<void> {
  patchGhosttyRenderLoop();
  ghosttyInitPromise ||= initGhostty();
  await ghosttyInitPromise;
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

function clipboardEventHasFileOrImage(event: ClipboardEvent): boolean {
  const clipboardData = event.clipboardData;
  if (!clipboardData) {
    return false;
  }

  if (clipboardData.files.length > 0 || Array.from(clipboardData.types).includes("Files")) {
    return true;
  }

  return Array.from(clipboardData.items).some(
    (item) => item.kind === "file" || item.type.startsWith("image/"),
  );
}

async function readTextForTerminalPaste(event?: ClipboardEvent): Promise<string> {
  const clipboardText = event?.clipboardData?.getData("text/plain") ?? "";
  if (clipboardText && event && !clipboardEventHasFileOrImage(event)) {
    return clipboardText;
  }

  try {
    const paste = await desktop().readTerminalClipboardPaste();
    if (paste.text) {
      return paste.text;
    }
  } catch (error) {
    logTerminalUi("terminal-clipboard-ipc-error", () => ({
      message: error instanceof Error ? error.message : String(error),
    }));
  }

  if (clipboardText) {
    return clipboardText;
  }

  try {
    return (await navigator.clipboard?.readText?.()) ?? "";
  } catch {
    return "";
  }
}

function pasteClipboardIntoTerminal(runtime: TerminalRuntime, event?: ClipboardEvent): void {
  void readTextForTerminalPaste(event).then((text) => {
    if (!text) {
      return;
    }
    runtime.term.clearSelection();
    runtime.term.paste(text);
  });
}

function copyTerminalSelection(term: Terminal, options?: { clearSelection?: boolean }): void {
  const selection = term.getSelection();
  if (!selection) {
    return;
  }

  void copyTextToClipboard(selection).catch((error) => {
    logTerminalUi("terminal-selection-copy-error", () => ({
      message: error instanceof Error ? error.message : String(error),
    }));
  });

  if (options?.clearSelection) {
    term.clearSelection();
  }
}

function handleTerminalClipboardShortcut(
  runtime: TerminalRuntime,
  action: TerminalClipboardShortcutAction,
): void {
  if (action === "paste") {
    pasteClipboardIntoTerminal(runtime);
    return;
  }

  copyTerminalSelection(runtime.term, { clearSelection: action === "cut" });
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
let terminalAppearance = defaultTerminalAppearanceConfig(desktopPlatform);

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

function applyTerminalAppearanceToHost(host: HTMLDivElement, term: Terminal): void {
  host.style.fontFamily = terminalAppearance.fontFamily;
  host.style.fontKerning = "none";
  host.style.fontVariantLigatures = "none";
  host.style.textRendering = "geometricPrecision";
  term.options.fontFamily = terminalAppearance.fontFamily;
  term.options.fontSize = terminalAppearance.fontSize;
  term.renderer?.remeasureFont();
}

function refreshTerminalThemes(): void {
  for (const runtime of terminalRuntimeRegistry.values()) {
    applyTerminalTheme(runtime.term);
  }
}

function refreshConnectedTerminalMetrics(): void {
  for (const runtime of terminalRuntimeRegistry.values()) {
    applyTerminalAppearanceToHost(runtime.host, runtime.term);
    if (runtime.host.isConnected) {
      runtime.fitAddon.fit();
    }
  }
}

if (typeof window.matchMedia === "function") {
  window
    .matchMedia("(prefers-color-scheme: dark)")
    .addEventListener("change", refreshTerminalThemes);
}

if (typeof document.fonts?.addEventListener === "function") {
  document.fonts.addEventListener("loadingdone", refreshConnectedTerminalMetrics);
}
if (typeof document.fonts?.ready?.then === "function") {
  void document.fonts.ready.then(() => {
    refreshConnectedTerminalMetrics();
  });
}

function formatTerminalChunk(data: string, limit = 120): string {
  const encoded = JSON.stringify(data);
  if (encoded.length <= limit) {
    return encoded;
  }
  return `${encoded.slice(0, limit)}...`;
}

function clampTerminalViewportY(term: Terminal, viewportY: number): number {
  return Math.max(0, Math.min(term.getScrollbackLength(), viewportY));
}

function writeTerminalChunk(term: Terminal, chunk: string): void {
  boostTerminalRenderLoop(term);
  const previousViewportY = term.getViewportY();
  if (previousViewportY <= 0) {
    term.write(chunk);
    return;
  }

  const previousScrollbackLength = term.getScrollbackLength();
  term.write(chunk);

  const nextScrollbackLength = term.getScrollbackLength();
  const nextViewportY = clampTerminalViewportY(
    term,
    previousViewportY + nextScrollbackLength - previousScrollbackLength,
  );
  term.scrollToLine(nextViewportY);
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
    fontSize: terminalAppearance.fontSize,
    fontFamily: terminalAppearance.fontFamily,
    theme: getTerminalTheme(),
  });
  setTerminalRenderLoopActive(term, false);
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

    const terminalClipboardAction = terminalClipboardShortcutActionForKeyEvent(event);
    if (terminalClipboardAction) {
      logTerminalUi("term-clipboard-shortcut", () => ({
        paneId,
        sessionId: runtime.sessionId,
        action: terminalClipboardAction,
      }));
      handleTerminalClipboardShortcut(runtime, terminalClipboardAction);
      return true;
    }

    return false;
  });
  host.addEventListener(
    "wheel",
    () => {
      boostTerminalRenderLoop(term, 300);
    },
    true,
  );
  host.tabIndex = 0;
  host.setAttribute("spellcheck", "false");
  applyTerminalAppearanceToHost(host, term);
  scrubTerminalSurface(host);
  fitAddon.fit();

  const writeQueue: string[] = [];
  let writeDrainScheduled = false;
  const drainWriteQueue = () => {
    let processedChunks = 0;
    while (processedChunks < MAX_TERMINAL_WRITE_CHUNKS_PER_TICK) {
      const chunk = takeStringChunk(writeQueue, MAX_TERMINAL_WRITE_CHUNK);
      if (!chunk) {
        return;
      }
      writeTerminalChunk(term, chunk);
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
      boostTerminalRenderLoop(term);
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

  host.addEventListener(
    "paste",
    (event) => {
      event.preventDefault();
      event.stopPropagation();
      logTerminalUi("term-paste-event", () => ({
        paneId,
        sessionId: runtime.sessionId,
        hasFileOrImage: clipboardEventHasFileOrImage(event),
        hasText: Boolean(event.clipboardData?.getData("text/plain")),
      }));
      pasteClipboardIntoTerminal(runtime, event);
    },
    true,
  );

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
    trackRendererDebugRate("term:onResize", {
      paneId,
      sessionId: runtime.sessionId,
      cols,
      rows,
    });
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
  setTerminalRenderLoopActive(runtime.term, true);
  requestAnimationFrame(() => {
    if (runtime.host.isConnected) {
      runtime.fitAddon.fit();
    }
  });
}

function refitConnectedTerminalRuntimes(): void {
  trackRendererDebugRate("term:refit-connected", {
    runtimes: terminalRuntimeRegistry.size,
  });
  for (const runtime of terminalRuntimeRegistry.values()) {
    if (!runtime.host.isConnected) {
      continue;
    }
    runtime.fitAddon.fit();
  }
}

function workspaceLayoutFitSignature(snapshot: WorkspaceSnapshot): string {
  const orderedColumnIds = [
    snapshot.pinnedLeftColumnId,
    ...snapshot.centerColumnIds,
    snapshot.pinnedRightColumnId,
  ].filter((columnId): columnId is string => columnId !== null);
  const seenColumnIds = new Set<string>();
  const parts = [
    `left:${snapshot.pinnedLeftColumnId ?? ""}`,
    `center:${snapshot.centerColumnIds.join(",")}`,
    `right:${snapshot.pinnedRightColumnId ?? ""}`,
  ];

  for (const columnId of orderedColumnIds) {
    if (seenColumnIds.has(columnId)) {
      continue;
    }
    seenColumnIds.add(columnId);
    const column = snapshot.columns[columnId];
    if (!column) {
      continue;
    }
    parts.push(
      [
        column.id,
        column.pinned ?? "",
        column.widthPx,
        column.paneIds.join(","),
        column.heightFractions.map((value) => value.toFixed(6)).join(","),
      ].join("|"),
    );
  }

  return parts.join("||");
}

function detachTerminalRuntime(runtime: TerminalRuntime): void {
  const activeElement = document.activeElement;
  if (activeElement instanceof HTMLElement && runtime.host.contains(activeElement)) {
    activeElement.blur();
  }
  setTerminalRenderLoopActive(runtime.term, false);
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
  setTerminalRenderLoopActive(runtime.term, false);
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

function wheelDeltaPixels(delta: number, deltaMode: number, pageSizePx: number): number {
  if (deltaMode === WheelEvent.DOM_DELTA_LINE) {
    return delta * WHEEL_LINE_DELTA_PX;
  }
  if (deltaMode === WheelEvent.DOM_DELTA_PAGE) {
    return delta * pageSizePx;
  }
  return delta;
}

function horizontalWheelFocusDelta(
  event: React.WheelEvent<HTMLElement>,
  pageSizePx: number,
): number {
  const horizontalDelta = event.deltaX || (event.shiftKey ? event.deltaY : 0);
  const verticalDelta = event.shiftKey ? 0 : event.deltaY;
  const horizontalPx = wheelDeltaPixels(horizontalDelta, event.deltaMode, pageSizePx);
  const verticalPx = wheelDeltaPixels(verticalDelta, event.deltaMode, pageSizePx);

  if (Math.abs(horizontalPx) < 12) {
    return 0;
  }
  if (Math.abs(horizontalPx) <= Math.abs(verticalPx) * 1.2) {
    return 0;
  }
  return horizontalPx;
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

  if (pane.type === "shell") {
    const target = paneElement.querySelector<HTMLElement>("textarea, [contenteditable='true']");
    if (!target) {
      return false;
    }
    try {
      target.focus({ preventScroll: true });
    } catch {
      target.focus();
    }
    return document.activeElement === target;
  }

  const selector =
    pane.type === "note"
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

function schedulePaneFocusRetry(paneId: string, pane: PaneState): () => void {
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
}

function mergeWorkspace(
  workspaces: WorkspaceSummary[],
  workspace: WorkspaceSummary,
): WorkspaceSummary[] {
  const next = workspaces.map((item) => (item.id === workspace.id ? workspace : item));
  return next.some((item) => item.id === workspace.id) ? next : [...next, workspace];
}

function mergeProjectRoot(
  roots: BootstrapPayload["projectRoots"],
  root: BootstrapPayload["projectRoots"][number],
): BootstrapPayload["projectRoots"] {
  const next = roots.map((item) => (item.id === root.id ? root : item));
  return next.some((item) => item.id === root.id) ? next : [...next, root];
}

function App(): React.ReactElement {
  const [bootstrap, setBootstrap] = useState<BootstrapPayload>({
    projectRoots: [],
    workspaces: [],
    terminalAppearance: defaultTerminalAppearanceConfig(desktopPlatform),
  });
  const [details, setDetails] = useState<Record<string, WorkspaceDetail>>({});
  const [activeWorkspaceId, setActiveWorkspaceId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loadingWorkspaceId, setLoadingWorkspaceId] = useState<string | null>(null);
  const [showRootForm, setShowRootForm] = useState(false);
  const [rootPathInput, setRootPathInput] = useState("");
  const [openRootMenuId, setOpenRootMenuId] = useState<string | null>(null);
  const [openWorkspaceMenuId, setOpenWorkspaceMenuId] = useState<string | null>(null);
  const [editingRootId, setEditingRootId] = useState<string | null>(null);
  const [editingRootDisplayName, setEditingRootDisplayName] = useState("");
  const [editingWorkspaceId, setEditingWorkspaceId] = useState<string | null>(null);
  const [editingWorkspaceDisplayName, setEditingWorkspaceDisplayName] = useState("");
  const [creatingWorkspaceRootId, setCreatingWorkspaceRootId] = useState<string | null>(null);
  const [keyboardNavigationRequest, setKeyboardNavigationRequest] =
    useState<KeyboardNavigationRequest | null>(null);

  const detailsRef = useRef(details);
  const sessionListenersRef = useRef<Map<string, Set<(event: SessionEvent) => void>>>(new Map());
  const snapshotTimersRef = useRef<Map<string, number>>(new Map());
  const activeWorkspaceIdRef = useRef<string | null>(null);
  const activeDetail = activeWorkspaceId ? details[activeWorkspaceId] ?? null : null;
  const workspaceShortcutLabels = useMemo(() => {
    const labels = new Map<string, string>();
    for (const target of workspaceShortcutTargets(bootstrap.projectRoots, bootstrap.workspaces)) {
      const label = workspaceShortcutLabel(target.index);
      if (label) {
        labels.set(target.workspace.id, label);
      }
    }
    return labels;
  }, [bootstrap.projectRoots, bootstrap.workspaces]);

  useEffect(() => {
    detailsRef.current = details;
  }, [details]);

  useEffect(() => {
    activeWorkspaceIdRef.current = activeWorkspaceId;
  }, [activeWorkspaceId]);

  useEffect(() => {
    terminalAppearance = bootstrap.terminalAppearance;
    refreshConnectedTerminalMetrics();
  }, [bootstrap.terminalAppearance]);

  useEffect(() => {
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      if (target?.closest("[data-nav-menu-root='true']")) {
        return;
      }
      setOpenRootMenuId(null);
      setOpenWorkspaceMenuId(null);
    };

    window.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
    };
  }, []);

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
      void desktop().saveSnapshot(workspaceId, snapshot).catch((err) => {
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
        const detail = await desktop().openWorkspace(workspaceId, currentViewportWidth());
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
    void desktop()
      .getBootstrap()
      .then((payload) => {
        setBootstrap(payload);
      })
      .catch((err) => {
        setError(err instanceof Error ? err.message : String(err));
      });
  }, []);

  useEffect(() => {
    const forwarder = (message: string, details: Record<string, unknown>) => {
      if (debugTerminal || debugMessageRates) {
        console.log("[terminal-ui]", message, details);
      }
    };
    forwardTerminalUiDebug = forwarder;
    const unsubscribe = desktop().onWorkspaceEvent((message) => {
      trackRendererDebugRate(`ws:recv:${message.type}`);

      if (message.type === "nav-updated") {
        const payload = message.payload as BootstrapPayload;
        setBootstrap(payload);
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
      unsubscribe();
    };
  }, [debugMessageRates, debugTerminal, emitSession]);

  const browseRoot = useCallback(async () => {
    try {
      const path = await desktop().pickDirectory(rootPathInput || undefined);
      if (path) {
        setRootPathInput(path);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [rootPathInput]);

  const submitRoot = useCallback(async () => {
    try {
      await desktop().addProjectRoot(rootPathInput);
      setShowRootForm(false);
      setRootPathInput("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [rootPathInput]);

  const createWorkspace = useCallback(async (rootId: string) => {
    setCreatingWorkspaceRootId(rootId);
    try {
      const workspace = await desktop().createWorkspace({
        rootId,
      });
      setBootstrap((current) => ({
        ...current,
        workspaces: mergeWorkspace(current.workspaces, workspace),
      }));
      await loadWorkspace(workspace.id);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setCreatingWorkspaceRootId((current) => (current === rootId ? null : current));
    }
  }, [loadWorkspace]);

  const forgetWorkspace = useCallback(async (workspaceId: string) => {
    const confirmed = window.confirm("Forget this workspace from JJ and the app?");
    if (!confirmed) {
      return;
    }

    try {
      await desktop().forgetWorkspace(workspaceId);
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

  const deleteAndForgetWorkspace = useCallback(async (workspace: WorkspaceSummary) => {
    const confirmed = window.confirm(
      `Delete the workspace directory and forget "${workspace.displayName}" from JJ and the app?\n\n${workspace.workspacePath}`,
    );
    if (!confirmed) {
      return;
    }

    try {
      await desktop().deleteAndForgetWorkspace(workspace.id);
      setDetails((current) => {
        const next = { ...current };
        delete next[workspace.id];
        return next;
      });
      if (activeWorkspaceId === workspace.id) {
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
      await desktop().removeProjectRoot(rootId);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const startRootRename = useCallback((root: BootstrapPayload["projectRoots"][number]) => {
    setEditingRootId(root.id);
    setEditingRootDisplayName(root.displayName);
    setOpenRootMenuId(null);
  }, []);

  const startWorkspaceRename = useCallback((workspace: WorkspaceSummary) => {
    setEditingWorkspaceId(workspace.id);
    setEditingWorkspaceDisplayName(workspace.displayName);
    setOpenWorkspaceMenuId(null);
  }, []);

  const renameProjectRoot = useCallback(async (rootId: string, displayName: string) => {
    const nextDisplayName = displayName.trim();
    if (!nextDisplayName) {
      setError("Display name cannot be empty");
      return;
    }

    try {
      const root = await desktop().updateProjectRootDisplayName(rootId, nextDisplayName);
      setBootstrap((current) => ({
        ...current,
        projectRoots: mergeProjectRoot(current.projectRoots, root),
      }));
      setEditingRootId(null);
      setEditingRootDisplayName("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const renameWorkspace = useCallback(async (workspaceId: string, displayName: string) => {
    const nextDisplayName = displayName.trim();
    if (!nextDisplayName) {
      setError("Display name cannot be empty");
      return;
    }

    try {
      const workspace = await desktop().updateWorkspaceDisplayName(workspaceId, nextDisplayName);
      setBootstrap((current) => ({
        ...current,
        workspaces: mergeWorkspace(current.workspaces, workspace),
      }));
      setDetails((current) => {
        const detail = current[workspaceId];
        if (!detail) {
          return current;
        }
        return {
          ...current,
          [workspaceId]: {
            ...detail,
            workspace,
          },
        };
      });
      setEditingWorkspaceId(null);
      setEditingWorkspaceDisplayName("");
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, []);

  const createTerminalSession = useCallback(
    async (workspaceId: string, paneId: string, kind: TerminalKind, cols = 120, rows = 32) => {
      const session = await withTimeout(
        desktop().createTerminalSession({
          workspaceId,
          paneId,
          kind,
          cols,
          rows,
        }),
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
      setKeyboardNavigationRequest({
        workspaceId,
        paneId: null,
        nonce: Date.now(),
      });
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
      const workspaceShortcutIndex = workspaceShortcutIndexForAction(action);
      if (workspaceShortcutIndex !== null) {
        const target = workspaceShortcutTargets(bootstrap.projectRoots, bootstrap.workspaces)
          .find((item) => item.index === workspaceShortcutIndex)?.workspace ?? null;
        if (!target || target.id === activeWorkspaceId) {
          return;
        }

        setKeyboardNavigationRequest({
          workspaceId: target.id,
          paneId: null,
          nonce: Date.now(),
        });
        void loadWorkspace(target.id);
        return;
      }

      const detail = activeWorkspaceId ? detailsRef.current[activeWorkspaceId] ?? null : null;
      const activePaneId = detail?.snapshot.activePaneId ?? null;

      switch (action) {
        case "open-shell-pane":
        case "open-codex-pane":
        case "open-pi-pane":
        case "open-nvim-pane":
        case "open-jjui-pane":
        case "open-browser-pane":
        case "open-diff-pane": {
          if (!detail) {
            return;
          }

          if (action === "open-browser-pane") {
            addPaneToWorkspace(detail.workspace.id, "browser");
            return;
          }

          if (action === "open-diff-pane") {
            addPaneToWorkspace(detail.workspace.id, "diff");
            return;
          }

          const terminalKind =
            action === "open-shell-pane"
              ? "shell"
              : action === "open-codex-pane"
                ? "codex"
                : action === "open-pi-pane"
                  ? "pi"
                  : action === "open-nvim-pane"
                    ? "nvim"
                    : "jjui";
          addPaneToWorkspace(detail.workspace.id, "shell", terminalKind);
          return;
        }
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
              desktop().closeTerminal(payload.sessionId);
            }
            destroyTerminalRuntime(activePane.id);
          }
          if (activePane.type === "browser") {
            desktop().destroyBrowserPane(activePane.id);
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
    [
      activeWorkspaceId,
      addPaneToWorkspace,
      bootstrap.projectRoots,
      bootstrap.workspaces,
      loadWorkspace,
      mutateWorkspace,
    ],
  );

  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) {
        return;
      }
      if (showRootForm || editingRootId || editingWorkspaceId) {
        return;
      }
      if (!event.ctrlKey || event.metaKey) {
        return;
      }
      const action = appShortcutActionForKeyEvent(event);
      if (action) {
        event.preventDefault();
        event.stopPropagation();
        invokeAppShortcut(action);
        return;
      }
    };

    window.addEventListener("keydown", handleShortcut, true);
    return () => {
      window.removeEventListener("keydown", handleShortcut, true);
    };
  }, [activeWorkspaceId, bootstrap.workspaces, editingRootId, editingWorkspaceId, invokeAppShortcut, loadWorkspace, mutateWorkspace, showRootForm]);

  useEffect(() => {
    const orbitWindow = window as WorkspaceOrbitWindow;
    orbitWindow.__workspaceOrbitHandleClosePane = () => invokeAppShortcut("close-pane");
    orbitWindow.__workspaceOrbitInvokeShortcut = invokeAppShortcut;
    const handleNativeShortcutEvent = (event: Event) => {
      const shortcutEvent = event as ShortcutBridgeEvent;
      if (typeof shortcutEvent.detail !== "string") {
        return;
      }
      invokeAppShortcut(shortcutEvent.detail);
    };

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

    window.addEventListener("octty-shortcut", handleNativeShortcutEvent as EventListener);
    window.addEventListener("keydown", handleClosePaneKey, true);
    return () => {
      window.removeEventListener("octty-shortcut", handleNativeShortcutEvent as EventListener);
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
                  <div className="project-title-row">
                    {editingRootId === root.id ? (
                      <input
                        autoFocus
                        className="inline-rename-input"
                        value={editingRootDisplayName}
                        onChange={(event) => setEditingRootDisplayName(event.target.value)}
                        onClick={(event) => event.stopPropagation()}
                        onKeyDown={(event) => {
                          if (event.key === "Enter") {
                            event.preventDefault();
                            event.currentTarget.dataset.skipCommit = "0";
                            event.currentTarget.blur();
                          }
                          if (event.key === "Escape") {
                            event.preventDefault();
                            event.currentTarget.dataset.skipCommit = "1";
                            setEditingRootId(null);
                            setEditingRootDisplayName("");
                            event.currentTarget.blur();
                          }
                        }}
                        onBlur={(event) => {
                          if (event.currentTarget.dataset.skipCommit === "1") {
                            event.currentTarget.dataset.skipCommit = "0";
                            return;
                          }
                          if (editingRootDisplayName.trim() === root.displayName) {
                            setEditingRootId(null);
                            setEditingRootDisplayName("");
                            return;
                          }
                          void renameProjectRoot(root.id, editingRootDisplayName);
                        }}
                      />
                    ) : (
                      <span
                        className="project-title"
                        onDoubleClick={(event) => {
                          event.stopPropagation();
                          startRootRename(root);
                        }}
                      >
                        {root.displayName}
                      </span>
                    )}
                    <div className="inline-menu-root" data-nav-menu-root="true">
                      <button
                        className="sidebar-inline-button inline-menu-trigger"
                        aria-label={`Project actions for ${root.displayName}`}
                        onClick={(event) => {
                          event.stopPropagation();
                          setOpenWorkspaceMenuId(null);
                          setOpenRootMenuId((current) => (current === root.id ? null : root.id));
                        }}
                      >
                        ...
                      </button>
                      {openRootMenuId === root.id && (
                        <div className="inline-menu">
                          <button
                            className="inline-menu-item"
                            disabled={creatingWorkspaceRootId === root.id}
                            onClick={(event) => {
                              event.stopPropagation();
                              setOpenRootMenuId(null);
                              void createWorkspace(root.id);
                            }}
                          >
                            {creatingWorkspaceRootId === root.id ? "Creating..." : "New Workspace"}
                          </button>
                          <button
                            className="inline-menu-item"
                            onClick={(event) => {
                              event.stopPropagation();
                              startRootRename(root);
                            }}
                          >
                            Rename
                          </button>
                          <button
                            className="inline-menu-item"
                            onClick={(event) => {
                              event.stopPropagation();
                              setOpenRootMenuId(null);
                              void removeProjectRoot(root.id);
                            }}
                          >
                            Remove
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              </div>

              <div className="workspace-list">
                {bootstrap.workspaces
                  .filter((workspace) => workspace.rootId === root.id)
                  .map((workspace) => {
                    const attentionClassName = agentAttentionClassName(workspace.agentAttentionState);
                    const attentionLabel = agentAttentionLabel(workspace.agentAttentionState);
                    const statusBadges = workspaceStatusBadges(workspace);
                    const shortcutLabel = workspaceShortcutLabels.get(workspace.id) ?? null;

                    return (
                      <div
                        key={workspace.id}
                        className={`workspace-item ${workspace.id === activeWorkspaceId ? "selected" : ""} ${hasRecordedWorkspacePath(workspace.workspacePath) ? "" : "unavailable"}`}
                      >
                        <div
                          className="workspace-select"
                          role="button"
                          tabIndex={0}
                          onClick={() => {
                            if (!hasRecordedWorkspacePath(workspace.workspacePath)) {
                              setError(
                                `JJ reports no recorded path for workspace "${workspace.workspaceName}". Forget it in JJ or reopen it from a real workspace directory.`,
                              );
                              return;
                            }
                            void loadWorkspace(workspace.id);
                          }}
                          onKeyDown={(event) => {
                            if (event.key !== "Enter" && event.key !== " ") {
                              return;
                            }
                            event.preventDefault();
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
                            <div className="workspace-title-row">
                              <span
                                className={`agent-attention-dot workspace-attention-dot ${attentionClassName ?? "workspace-attention-dot-null"}`}
                                title={attentionLabel ?? "No activity"}
                                aria-hidden="true"
                              />
                              {editingWorkspaceId === workspace.id ? (
                                <input
                                  autoFocus
                                  className="inline-rename-input"
                                  value={editingWorkspaceDisplayName}
                                  onChange={(event) => setEditingWorkspaceDisplayName(event.target.value)}
                                  onClick={(event) => event.stopPropagation()}
                                  onKeyDown={(event) => {
                                    if (event.key === "Enter") {
                                      event.preventDefault();
                                      event.currentTarget.dataset.skipCommit = "0";
                                      event.currentTarget.blur();
                                    }
                                    if (event.key === "Escape") {
                                      event.preventDefault();
                                      event.currentTarget.dataset.skipCommit = "1";
                                      setEditingWorkspaceId(null);
                                      setEditingWorkspaceDisplayName("");
                                      event.currentTarget.blur();
                                    }
                                  }}
                                  onBlur={(event) => {
                                    if (event.currentTarget.dataset.skipCommit === "1") {
                                      event.currentTarget.dataset.skipCommit = "0";
                                      return;
                                    }
                                    if (editingWorkspaceDisplayName.trim() === workspace.displayName) {
                                      setEditingWorkspaceId(null);
                                      setEditingWorkspaceDisplayName("");
                                      return;
                                    }
                                    void renameWorkspace(workspace.id, editingWorkspaceDisplayName);
                                  }}
                                />
                              ) : (
                                <span
                                  className="workspace-name"
                                  onDoubleClick={(event) => {
                                    event.stopPropagation();
                                    startWorkspaceRename(workspace);
                                  }}
                                >
                                  {workspace.displayName}
                                </span>
                              )}
                              {statusBadges.map((badge) => (
                                <span
                                  key={badge.className}
                                  className={`workspace-state-bead ${badge.className} ${workspace.hasWorkingCopyChanges ? "changed" : "unchanged"}`}
                                  title={badge.title}
                                  aria-label={badge.title}
                                >
                                  {badge.label}
                                </span>
                              ))}
                              <div className="inline-menu-root" data-nav-menu-root="true">
                                <button
                                  className="sidebar-inline-button inline-menu-trigger"
                                  aria-label={`Workspace actions for ${workspace.displayName}`}
                                  onClick={(event) => {
                                    event.stopPropagation();
                                    setOpenRootMenuId(null);
                                    setOpenWorkspaceMenuId((current) =>
                                      current === workspace.id ? null : workspace.id,
                                    );
                                  }}
                                >
                                  ...
                                </button>
                              {openWorkspaceMenuId === workspace.id && (
                                <div className="inline-menu">
                                  <button
                                    className="inline-menu-item"
                                    onClick={(event) => {
                                      event.stopPropagation();
                                      startWorkspaceRename(workspace);
                                    }}
                                  >
                                    Rename
                                  </button>
                                  {workspace.workspaceName !== "default" && (
                                    <button
                                      className="inline-menu-item"
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        setOpenWorkspaceMenuId(null);
                                        void forgetWorkspace(workspace.id);
                                      }}
                                    >
                                      Forget
                                    </button>
                                  )}
                                  {workspace.workspaceName !== "default" && hasRecordedWorkspacePath(workspace.workspacePath) && (
                                    <button
                                      className="inline-menu-item"
                                      onClick={(event) => {
                                        event.stopPropagation();
                                        setOpenWorkspaceMenuId(null);
                                        void deleteAndForgetWorkspace(workspace);
                                      }}
                                    >
                                      Delete and forget
                                    </button>
                                  )}
                                </div>
                              )}
                              </div>
                            </div>
                          </div>
                          {(workspace.bookmarks.length > 0 || shortcutLabel) && (
                            <div className="workspace-item-row workspace-bookmark-row">
                              {workspace.bookmarks.length > 0 && (
                                <span className="workspace-branch-info">{workspaceBookmarkLabel(workspace)}</span>
                              )}
                              {shortcutLabel && (
                                <span className="workspace-shortcut-label">{`<${shortcutLabel}>`}</span>
                              )}
                            </div>
                          )}
                          <div className="workspace-badges">
                          {!hasRecordedWorkspacePath(workspace.workspacePath) && (
                            <span className="badge warning">missing path</span>
                          )}
                          {workspace.unreadNotes > 0 && (
                            <span className="badge">{workspace.unreadNotes} note</span>
                          )}
                        </div>
                      </div>
                      </div>
                    );
                  })}
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
          </div>
        </div>
      </aside>

      <main className="workspace-shell">
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
                  const note = await desktop().createNote(activeDetail.workspace.id, fileName);
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
                  const note = await desktop().saveNote(activeDetail.workspace.id, path, body);
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
                  await desktop().markNoteRead(activeDetail.workspace.id, path);
                }}
                onCreateSession={createTerminalSession}
                onFetchSession={(sessionId) => desktop().getSession(sessionId)}
                onSendSessionInput={(sessionId, data) => desktop().sendTerminalInput(sessionId, data)}
                onResizeSession={(sessionId, cols, rows) => desktop().resizeTerminal(sessionId, cols, rows)}
                onSetSessionFocus={(sessionId, focused) => desktop().focusTerminal(sessionId, focused)}
                onCloseSession={(sessionId) => desktop().closeTerminal(sessionId)}
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
  const handledNavigationRequestNonceRef = useRef<number | null>(null);
  const lastCenteredColumnIdRef = useRef<string | null>(null);
  const horizontalWheelFocusRef = useRef({ accumulatedPx: 0, resetTimer: 0 });
  const wheelNavigationNonceRef = useRef(Date.now());
  const [wheelNavigationRequest, setWheelNavigationRequest] =
    useState<KeyboardNavigationRequest | null>(null);
  const activeColumnId = detail.snapshot.activePaneId
    ? findPaneColumnId(detail.snapshot, detail.snapshot.activePaneId)
    : null;
  const layoutFitSignature = workspaceLayoutFitSignature(detail.snapshot);
  const navigationRequest =
    !keyboardNavigationRequest
      ? wheelNavigationRequest
      : !wheelNavigationRequest
        ? keyboardNavigationRequest
        : keyboardNavigationRequest.nonce >= wheelNavigationRequest.nonce
          ? keyboardNavigationRequest
          : wheelNavigationRequest;

  useEffect(() => {
    if (!activeColumnId || !centerScrollRef.current) {
      return;
    }
    if (lastCenteredColumnIdRef.current === activeColumnId) {
      return;
    }
    if (!detail.snapshot.centerColumnIds.includes(activeColumnId)) {
      return;
    }
    lastCenteredColumnIdRef.current = activeColumnId;

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

    trackRendererDebugRate("taskspace:snapshot-effect", {
      workspaceId: detail.workspace.id,
      panes: Object.keys(detail.snapshot.panes).length,
    });
    const frame = window.requestAnimationFrame(() => {
      refitConnectedTerminalRuntimes();
    });

    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [detail.workspace.id, isVisible, layoutFitSignature]);

  useEffect(() => {
    const unsubscribe = desktop().onBrowserEvent((event) => {
      if (event.type === "close" && event.payload.workspaceId === detail.workspace.id) {
        mutateSnapshot((snapshot) => {
          const pane = snapshot.panes[event.payload.paneId];
          if (!pane || pane.type !== "browser") {
            return snapshot;
          }
          return removePane(snapshot, pane.id);
        });
        return;
      }

      if (event.type !== "popup" || event.payload.workspaceId !== detail.workspace.id) {
        return;
      }

      mutateSnapshot((snapshot) => {
        const openerPane = snapshot.panes[event.payload.paneId];
        if (!openerPane || openerPane.type !== "browser") {
          return snapshot;
        }

        const openerColumnId = findPaneColumnId(snapshot, openerPane.id);
        if (!openerColumnId) {
          return snapshot;
        }

        const previousPaneIds = new Set(Object.keys(snapshot.panes));
        let next = addPane(
          snapshot,
          "browser",
          detail.workspace.workspacePath,
          "new-column",
          "shell",
          currentViewportWidth(),
        );
        const popupPane = Object.values(next.panes).find((candidate) => !previousPaneIds.has(candidate.id));
        if (!popupPane) {
          return snapshot;
        }

        next = updatePane(next, popupPane.id, (pane) => ({
          ...pane,
          title: event.payload.title || "Browser",
          payload: {
            ...(pane.payload as BrowserPanePayload),
            url: event.payload.url,
            title: event.payload.title || "Browser",
            zoomFactor: 1,
            pendingPopupId: event.payload.popupId,
          },
        }));
        return movePaneToColumn(next, popupPane.id, openerColumnId);
      });
    });

    return unsubscribe;
  }, [detail.workspace.id, detail.workspace.workspacePath, mutateSnapshot]);

  useEffect(() => {
    if (!isVisible || !navigationRequest) {
      return;
    }
    if (handledNavigationRequestNonceRef.current === navigationRequest.nonce) {
      return;
    }
    if (navigationRequest.workspaceId !== detail.workspace.id) {
      return;
    }

    const paneId = navigationRequest.paneId ?? detail.snapshot.activePaneId;
    if (!paneId) {
      return;
    }

    const pane = detail.snapshot.panes[paneId];
    if (!pane) {
      return;
    }

    handledNavigationRequestNonceRef.current = navigationRequest.nonce;

    return schedulePaneFocusRetry(paneId, pane);
  }, [detail.snapshot.activePaneId, detail.snapshot.panes, detail.workspace.id, isVisible, navigationRequest]);

  useEffect(() => {
    if (!isVisible || !detail.snapshot.activePaneId) {
      return;
    }

    const paneId = detail.snapshot.activePaneId;
    const pane = detail.snapshot.panes[paneId];
    if (!pane) {
      return;
    }

    return schedulePaneFocusRetry(paneId, pane);
  }, [detail.snapshot.activePaneId, detail.snapshot.panes, isVisible]);

  useEffect(() => {
    return () => {
      const resetTimer = horizontalWheelFocusRef.current.resetTimer;
      if (resetTimer) {
        window.clearTimeout(resetTimer);
      }
    };
  }, []);

  const focusPaneByHorizontalWheel = useCallback(
    (direction: -1 | 1): boolean => {
      const nextPaneId = cycleShortcutTarget(
        listPaneShortcutIds(detail.snapshot),
        detail.snapshot.activePaneId,
        direction,
      );
      if (!nextPaneId || nextPaneId === detail.snapshot.activePaneId) {
        return false;
      }

      activatePane(nextPaneId);
      wheelNavigationNonceRef.current = Math.max(Date.now(), wheelNavigationNonceRef.current + 1);
      setWheelNavigationRequest({
        workspaceId: detail.workspace.id,
        paneId: nextPaneId,
        nonce: wheelNavigationNonceRef.current,
      });
      return true;
    },
    [activatePane, detail.snapshot, detail.workspace.id],
  );

  const handleTaskspaceWheelCapture = useCallback(
    (event: React.WheelEvent<HTMLDivElement>) => {
      const focusDelta = horizontalWheelFocusDelta(
        event,
        Math.max(event.currentTarget.clientWidth, 1),
      );
      if (focusDelta === 0) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();

      const state = horizontalWheelFocusRef.current;
      if (state.resetTimer) {
        window.clearTimeout(state.resetTimer);
      }
      state.resetTimer = window.setTimeout(() => {
        state.accumulatedPx = 0;
        state.resetTimer = 0;
      }, HORIZONTAL_WHEEL_FOCUS_RESET_MS);

      const direction = focusDelta > 0 ? 1 : -1;
      if (state.accumulatedPx !== 0 && Math.sign(state.accumulatedPx) !== direction) {
        state.accumulatedPx = 0;
      }

      state.accumulatedPx += focusDelta;
      if (Math.abs(state.accumulatedPx) < HORIZONTAL_WHEEL_FOCUS_THRESHOLD_PX) {
        return;
      }

      if (focusPaneByHorizontalWheel(direction)) {
        state.accumulatedPx -= direction * HORIZONTAL_WHEEL_FOCUS_THRESHOLD_PX;
        if (state.accumulatedPx !== 0 && Math.sign(state.accumulatedPx) !== direction) {
          state.accumulatedPx = 0;
        }
        return;
      }

      state.accumulatedPx = 0;
    },
    [focusPaneByHorizontalWheel],
  );

  const closePane = useCallback(
    (pane: PaneState) => {
      if (pane.type === "shell") {
        const payload = pane.payload as TerminalPanePayload;
        if (payload.sessionId) {
          onCloseSession(payload.sessionId);
        }
        destroyTerminalRuntime(pane.id);
      }
      if (pane.type === "browser") {
        desktop().destroyBrowserPane(pane.id);
      }
      mutateSnapshot((snapshot) => removePane(snapshot, pane.id));
    },
    [mutateSnapshot, onCloseSession],
  );

  const focusActivePaneFromScrollContainer = useCallback((): boolean => {
    const paneId = detail.snapshot.activePaneId;
    if (!paneId) {
      return false;
    }

    const pane = detail.snapshot.panes[paneId];
    if (!pane) {
      return false;
    }

    const paneElement = document.querySelector<HTMLElement>(`[data-pane-id="${paneId}"]`);
    return Boolean(paneElement && focusPaneTarget(paneId, paneElement, pane));
  }, [detail.snapshot.activePaneId, detail.snapshot.panes]);

  const handleCenterScrollFocus = useCallback(
    (event: React.FocusEvent<HTMLDivElement>) => {
      if (event.target !== event.currentTarget) {
        return;
      }

      window.requestAnimationFrame(() => {
        focusActivePaneFromScrollContainer();
      });
    },
    [focusActivePaneFromScrollContainer],
  );

  const handleCenterScrollKeyDownCapture = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.target !== event.currentTarget) {
        return;
      }
      if (!focusActivePaneFromScrollContainer()) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();
    },
    [focusActivePaneFromScrollContainer],
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
    <div className="taskspace" onWheelCapture={handleTaskspaceWheelCapture}>
      <aside
        className={`pinned-slot left ${detail.snapshot.pinnedLeftColumnId ? "" : "empty"}`}
      >
        {detail.snapshot.pinnedLeftColumnId
          ? renderColumn(detail.snapshot.pinnedLeftColumnId)
          : <div className="pin-placeholder">Fix left</div>}
      </aside>

      <section className="center-taskspace">
        <div
          className="center-column-scroll"
          ref={centerScrollRef}
          onFocus={handleCenterScrollFocus}
          onKeyDownCapture={handleCenterScrollKeyDownCapture}
        >
          <div className="center-column-strip">
            {detail.snapshot.centerColumnIds.length === 0 && (
              <div className="empty-stage compact">
                <p>Add a pane to start a column.</p>
              </div>
            )}

            {detail.snapshot.centerColumnIds.map((columnId) => renderColumn(columnId))}
          </div>
        </div>
      </section>

      <aside
        className={`pinned-slot right ${detail.snapshot.pinnedRightColumnId ? "" : "empty"}`}
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
            onClosePane={onClose}
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
            workspace={workspace}
            isActive={isActive}
            isVisible={isVisible}
            onFocus={onFocus}
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
  onClosePane,
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
  onClosePane: () => void;
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
  const onClosePaneRef = useRef(onClosePane);
  const onUpdatePaneRef = useRef(onUpdatePane);
  const pendingSessionStartRef = useRef<Promise<void> | null>(null);
  const resizeFrameRef = useRef<number | null>(null);
  const restoreRerenderTimerRef = useRef<number[]>([]);
  const lastSyncedSizeRef = useRef<{ sessionId: string; cols: number; rows: number } | null>(null);
  const lastObservedContainerSizeRef = useRef<{ width: number; height: number } | null>(null);
  const [statusText, setStatusText] = useState<string>(terminalStatusLabel(payload));

  useEffect(() => {
    payloadRef.current = payload;
    setStatusText(terminalStatusLabel(payload));
    if (!payload.sessionId) {
      lastSyncedSizeRef.current = null;
    }
  }, [payload.autoStart, payload.exitCode, payload.sessionId, payload.sessionState]);

  useEffect(() => {
    subscribeSessionRef.current = subscribeSession;
    onCreateSessionRef.current = onCreateSession;
    onFetchSessionRef.current = onFetchSession;
    onSendSessionInputRef.current = onSendSessionInput;
    onResizeSessionRef.current = onResizeSession;
    onSetSessionFocusRef.current = onSetSessionFocus;
    onClosePaneRef.current = onClosePane;
    onUpdatePaneRef.current = onUpdatePane;
  }, [
    onCreateSession,
    onClosePane,
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

  useEffect(() => {
    const runtime = runtimeRef.current;
    if (!runtime) {
      return;
    }
    setTerminalRenderLoopMode(runtime.term, isVisible ? (isActive ? "focused" : "visible") : "stopped");
  }, [isActive, isVisible, pane.id]);

  useEffect(() => {
    if (
      payload.sessionState !== "live" &&
      !payload.autoStart &&
      shouldCloseTerminalPaneOnExit(payload.exitCode)
    ) {
      onClosePaneRef.current();
    }
  }, [payload.autoStart, payload.exitCode, payload.sessionState]);

  const resetSurface = useCallback(() => {
    resetTerminalSurface(containerRef.current);
  }, []);

  const focusTerminalInput = useCallback((): boolean => {
    const container = containerRef.current;
    if (!container) {
      return false;
    }
    runtimeRef.current?.term.focus();
    const inputTarget = container.querySelector<HTMLElement>("textarea, [contenteditable='true']");
    if (inputTarget) {
      logTerminalUi("focus-terminal", {
        paneId: pane.id,
        target: describeElement(inputTarget),
        activeElement: describeElement(document.activeElement),
      });
      return document.activeElement === inputTarget;
    }

    const host = container.querySelector<HTMLElement>(".terminal-runtime-host");
    if (!host) {
      return false;
    }

    host.focus();
    logTerminalUi("focus-terminal", {
      paneId: pane.id,
      target: describeElement(host),
      activeElement: describeElement(document.activeElement),
    });
    return Boolean(document.activeElement instanceof HTMLElement && host.contains(document.activeElement));
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

  const cancelRestoreRerender = useCallback(() => {
    if (restoreRerenderTimerRef.current.length > 0) {
      for (const handle of restoreRerenderTimerRef.current) {
        window.clearTimeout(handle);
      }
      restoreRerenderTimerRef.current = [];
    }
  }, []);

  const scheduleFitAndResizeSync = useCallback((options?: { forceReflow?: boolean }) => {
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

      const containerWidth = container.clientWidth;
      const containerHeight = container.clientHeight;
      const lastObservedContainerSize = lastObservedContainerSizeRef.current;
      if (
        !options?.forceReflow &&
        lastObservedContainerSize &&
        lastObservedContainerSize.width === containerWidth &&
        lastObservedContainerSize.height === containerHeight
      ) {
        return;
      }
      lastObservedContainerSizeRef.current = {
        width: containerWidth,
        height: containerHeight,
      };

      const previousCols = Math.max(20, runtime.term.cols ?? 120);
      const previousRows = Math.max(5, runtime.term.rows ?? 32);
      if (options?.forceReflow) {
        const originalFontSize = runtime.term.options.fontSize;
        runtime.term.options.fontSize = originalFontSize + 0.01;
        runtime.term.options.fontSize = originalFontSize;
      }

      runtime.fitAddon.fit();

      let cols = Math.max(20, runtime.term.cols ?? 120);
      let rows = Math.max(5, runtime.term.rows ?? 32);
      if (options?.forceReflow && cols === previousCols && rows === previousRows) {
        const nudgedCols = cols > 20 ? cols - 1 : cols + 1;
        runtime.term.resize(nudgedCols, rows);
        runtime.term.resize(cols, rows);
        cols = Math.max(20, runtime.term.cols ?? cols);
        rows = Math.max(5, runtime.term.rows ?? rows);
      }

      const activePayload = payloadRef.current;
      if (activePayload.sessionState !== "live" || !activePayload.sessionId) {
        return;
      }

      const lastSyncedSize = lastSyncedSizeRef.current;
      if (
        lastSyncedSize &&
        lastSyncedSize.sessionId === activePayload.sessionId &&
        lastSyncedSize.cols === cols &&
        lastSyncedSize.rows === rows
      ) {
        return;
      }
      lastSyncedSizeRef.current = {
        sessionId: activePayload.sessionId,
        cols,
        rows,
      };
      trackRendererDebugRate("term:sync-resize", {
        paneId: pane.id,
        sessionId: activePayload.sessionId,
        cols,
        rows,
        forceReflow: options?.forceReflow ?? false,
      });
      onResizeSessionRef.current(activePayload.sessionId, cols, rows);
    });
  }, [pane.id]);

  const scheduleRestoreRerender = useCallback((sessionId: string, kind: TerminalKind) => {
    const rerenderMode = terminalRestoreRerenderMode(kind);
    if (!rerenderMode) {
      return;
    }

    cancelRestoreRerender();
    const queueRestorePass = (delayMs: number) => {
      const handle = window.setTimeout(() => {
        restoreRerenderTimerRef.current = restoreRerenderTimerRef.current.filter(
          (timerHandle) => timerHandle !== handle,
        );
        if (payloadRef.current.sessionId !== sessionId) {
          return;
        }
        if (rerenderMode === "resize") {
          scheduleFitAndResizeSync({ forceReflow: true });
        }
      }, delayMs);
      restoreRerenderTimerRef.current.push(handle);
    };

    for (const delayMs of [40, 120, 260, 600, 1200]) {
      queueRestorePass(delayMs);
    }

    if (typeof document.fonts?.ready?.then === "function") {
      void document.fonts.ready.then(() => {
        if (payloadRef.current.sessionId === sessionId) {
          scheduleFitAndResizeSync({ forceReflow: true });
        }
      });
    }
  }, [cancelRestoreRerender, scheduleFitAndResizeSync]);

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
      trackRendererDebugRate("term:container-resize", { paneId: pane.id });
      scheduleFitAndResizeSync();
    });
    observer.observe(container);
    scheduleFitAndResizeSync();

    return () => {
      observer.disconnect();
    };
  }, [isVisible, pane.id, scheduleFitAndResizeSync]);

  const startSession = useCallback(
    (kind: TerminalKind, options?: { focus?: boolean; rerenderAfterRestore?: boolean }) => {
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
          lastSyncedSizeRef.current = null;
          onResizeSessionRef.current(session.id, cols, rows);
          lastSyncedSizeRef.current = { sessionId: session.id, cols, rows };
          if (options?.rerenderAfterRestore) {
            scheduleRestoreRerender(session.id, session.kind);
          }
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
    [currentTerminalSize, focusTerminalInput, pane.id, scheduleRestoreRerender, workspace.id],
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
      lastObservedContainerSizeRef.current = null;
      cancelRestoreRerender();
      runtime.sendInput = (sessionId, data) => onSendSessionInputRef.current(sessionId, data);
      runtime.resizeSession = (sessionId, cols, rows) =>
        onResizeSessionRef.current(sessionId, cols, rows);
      runtime.onExit = (exitCode) => {
        if (shouldCloseTerminalPaneOnExit(exitCode)) {
          onClosePaneRef.current();
          return;
        }
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
      setTerminalRenderLoopMode(runtime.term, isActive ? "focused" : "visible");
      terminalRef.current = runtime.term;

      if (payload.sessionId) {
        if (runtime.sessionId === payload.sessionId) {
          setStatusText(payload.sessionState === "live" ? "live" : terminalStatusLabel(payload));
          if (isVisible) {
            const { cols, rows } = currentTerminalSize();
            lastSyncedSizeRef.current = null;
            onResizeSessionRef.current(payload.sessionId, cols, rows);
            lastSyncedSizeRef.current = { sessionId: payload.sessionId, cols, rows };
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
              lastSyncedSizeRef.current = null;
              onResizeSessionRef.current(session.id, cols, rows);
              lastSyncedSizeRef.current = { sessionId: session.id, cols, rows };
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
            scheduleRestoreRerender(session.id, session.kind);
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
          await startSession(payload.kind, {
            focus: false,
            rerenderAfterRestore: true,
          });
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
      cancelRestoreRerender();
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
    cancelRestoreRerender,
    resetSurface,
    scheduleRestoreRerender,
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
      {payload.sessionState !== "live" &&
        !payload.autoStart &&
        shouldShowTerminalRestart(payload.exitCode) && (
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
  workspace,
  isVisible,
  onFocus,
  onUpdatePane,
}: {
  pane: PaneState;
  workspace: WorkspaceSummary;
  isActive: boolean;
  isVisible: boolean;
  onFocus: () => void;
  onUpdatePane: (updater: (pane: PaneState) => PaneState) => void;
}): React.ReactElement {
  const payload = pane.payload as BrowserPanePayload;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const findInputRef = useRef<HTMLInputElement | null>(null);
  const surfaceRef = useRef<HTMLDivElement | null>(null);
  const initialBrowserUrlRef = useRef(payload.url);
  const initialBrowserZoomRef = useRef(payload.zoomFactor);
  const onFocusRef = useRef(onFocus);
  const onUpdatePaneRef = useRef(onUpdatePane);
  const lastBoundsRef = useRef<BrowserViewBounds | null>(null);
  const syncFrameRef = useRef<number | null>(null);
  const urlFocusedRef = useRef(false);
  const [browserState, setBrowserState] = useState<BrowserViewState | null>(null);
  const [urlValue, setUrlValue] = useState(payload.url);
  const [findOpen, setFindOpen] = useState(false);
  const [findValue, setFindValue] = useState("");
  const [findResult, setFindResult] = useState<BrowserFindResult | null>(null);
  const [download, setDownload] = useState<BrowserDownloadState | null>(null);
  const [statusText, setStatusText] = useState<string | null>(null);
  const zoomFactor = browserState?.zoomFactor ?? payload.zoomFactor ?? 1;
  const zoomLabel = useMemo(() => `${Math.round(zoomFactor * 100)}%`, [zoomFactor]);

  useEffect(() => {
    onFocusRef.current = onFocus;
    onUpdatePaneRef.current = onUpdatePane;
  }, [onFocus, onUpdatePane]);

  useEffect(() => {
    if (!urlFocusedRef.current) {
      setUrlValue(payload.url);
    }
  }, [payload.url]);

  const applyBrowserState = useCallback(
    (nextState: BrowserViewState) => {
      setBrowserState(nextState);
      setStatusText(nextState.errorText);
      if (!urlFocusedRef.current) {
        setUrlValue(nextState.url);
      }
      onUpdatePaneRef.current((current) => {
        const currentPayload = current.payload as BrowserPanePayload;
        const nextTitle = nextState.title || "Browser";
        if (
          current.title === nextTitle &&
          currentPayload.url === nextState.url &&
          currentPayload.title === nextState.title &&
          currentPayload.zoomFactor === nextState.zoomFactor &&
          currentPayload.pendingPopupId === null
        ) {
          return current;
        }
        return {
          ...current,
          title: nextTitle,
          payload: {
            ...currentPayload,
            url: nextState.url,
            title: nextState.title,
            zoomFactor: nextState.zoomFactor,
            pendingPopupId: null,
          },
        };
      });
    },
    [],
  );

  const hideNativeBrowser = useCallback(() => {
    lastBoundsRef.current = null;
    desktop().hideBrowserPane(pane.id);
  }, [pane.id]);

  const browserSurfaceGeometry = useCallback((): BrowserViewBounds | null => {
    const surface = surfaceRef.current;
    if (!surface || !isVisible || document.querySelector(".modal-backdrop")) {
      return null;
    }

    const rect = surface.getBoundingClientRect();
    const next = {
      x: rect.x,
      y: rect.y,
      width: rect.width,
      height: rect.height,
    };
    if (next.width < 40 || next.height < 40) {
      return null;
    }

    const clips: BrowserSurfaceRect[] = [{
      x: 0,
      y: 0,
      width: window.innerWidth,
      height: window.innerHeight,
    }];

    const scrollContainer = surface.closest(".center-column-scroll");
    if (scrollContainer) {
      const clip = scrollContainer.getBoundingClientRect();
      clips.push({
        x: clip.x,
        y: clip.y,
        width: clip.width,
        height: clip.height,
      });
    }

    const visible = visibleRectIntersection(next, clips);
    if (!visible || visible.width < 40 || visible.height < 40) {
      return null;
    }
    return {
      x: visible.x,
      y: visible.y,
      width: visible.width,
      height: visible.height,
      contentBounds: next,
    };
  }, [isVisible]);

  const syncBrowserBounds = useCallback(() => {
    syncFrameRef.current = null;
    const geometry = browserSurfaceGeometry();
    if (!geometry) {
      hideNativeBrowser();
      return;
    }

    const bounds = {
      x: Math.round(geometry.x),
      y: Math.round(geometry.y),
      width: Math.round(geometry.width),
      height: Math.round(geometry.height),
      contentBounds: geometry.contentBounds
        ? {
            x: Math.round(geometry.contentBounds.x),
            y: Math.round(geometry.contentBounds.y),
            width: Math.round(geometry.contentBounds.width),
            height: Math.round(geometry.contentBounds.height),
          }
        : undefined,
    };
    const previous = lastBoundsRef.current;
    if (
      previous &&
      previous.x === bounds.x &&
      previous.y === bounds.y &&
      previous.width === bounds.width &&
      previous.height === bounds.height &&
      previous.contentBounds?.x === bounds.contentBounds?.x &&
      previous.contentBounds?.y === bounds.contentBounds?.y &&
      previous.contentBounds?.width === bounds.contentBounds?.width &&
      previous.contentBounds?.height === bounds.contentBounds?.height
    ) {
      return;
    }
    lastBoundsRef.current = bounds;
    desktop().setBrowserBounds(pane.id, bounds);
  }, [browserSurfaceGeometry, hideNativeBrowser, pane.id]);

  const scheduleBoundsSync = useCallback(() => {
    if (syncFrameRef.current !== null) {
      return;
    }
    syncFrameRef.current = window.requestAnimationFrame(syncBrowserBounds);
  }, [syncBrowserBounds]);

  useEffect(() => {
    let disposed = false;
    void desktop()
      .ensureBrowserPane(
        workspace.id,
        pane.id,
        initialBrowserUrlRef.current,
        initialBrowserZoomRef.current,
        payload.pendingPopupId,
      )
      .then((state) => {
        if (!disposed) {
          applyBrowserState(state);
          scheduleBoundsSync();
        }
      })
      .catch((error) => {
        if (!disposed) {
          setStatusText(error instanceof Error ? error.message : String(error));
        }
      });

    return () => {
      disposed = true;
      hideNativeBrowser();
    };
  }, [
    applyBrowserState,
    hideNativeBrowser,
    pane.id,
    scheduleBoundsSync,
    workspace.id,
  ]);

  useEffect(() => {
    const unsubscribe = desktop().onBrowserEvent((event: BrowserEventEnvelope) => {
      if (
        "payload" in event &&
        event.payload &&
        "paneId" in event.payload &&
        event.payload.paneId !== pane.id
      ) {
        return;
      }

      if (event.type === "state" && event.payload.workspaceId === workspace.id) {
        applyBrowserState(event.payload);
        return;
      }

      if (event.type === "focus" && event.payload.workspaceId === workspace.id) {
        onFocusRef.current();
        return;
      }

      if (event.type === "shortcut" && event.payload.workspaceId === workspace.id) {
        if (event.payload.action === "focus-location") {
          inputRef.current?.focus();
          inputRef.current?.select();
          return;
        }
        if (event.payload.action === "focus-find") {
          setFindOpen(true);
          window.requestAnimationFrame(() => {
            findInputRef.current?.focus();
            findInputRef.current?.select();
          });
          return;
        }
        if (event.payload.action === "zoom-in") {
          void setZoom(clampBrowserZoom(zoomFactor + 0.1));
          return;
        }
        if (event.payload.action === "zoom-out") {
          void setZoom(clampBrowserZoom(zoomFactor - 0.1));
          return;
        }
        if (event.payload.action === "zoom-reset") {
          void setZoom(1);
        }
        return;
      }

      if (event.type === "find-result" && event.payload.workspaceId === workspace.id) {
        setFindResult(event.payload.result);
        return;
      }

      if (
        event.type === "download" &&
        event.payload.workspaceId === workspace.id &&
        event.payload.download.paneId === pane.id
      ) {
        setDownload(event.payload.download);
        window.setTimeout(() => {
          setDownload((current) => (current?.id === event.payload.download.id ? null : current));
        }, 4_000);
      }
    });
    return unsubscribe;
  }, [applyBrowserState, pane.id, workspace.id, zoomFactor]);

  const setZoom = useCallback(
    async (nextZoom: number) => {
      const state = await desktop().setBrowserZoom(pane.id, clampBrowserZoom(nextZoom));
      applyBrowserState(state);
    },
    [applyBrowserState, pane.id],
  );

  useEffect(() => {
    paneFocusRegistry.set(pane.id, () => {
      void desktop().focusBrowserPane(pane.id);
      return true;
    });
    return () => {
      if (paneFocusRegistry.has(pane.id)) {
        paneFocusRegistry.delete(pane.id);
      }
    };
  }, [pane.id]);

  useEffect(() => {
    if (!isVisible) {
      hideNativeBrowser();
      return;
    }

    const surface = surfaceRef.current;
    const observer =
      surface && typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(scheduleBoundsSync)
        : null;
    observer?.observe(surface!);

    const scrollContainer = surface?.closest(".center-column-scroll");
    window.addEventListener("resize", scheduleBoundsSync);
    window.addEventListener("scroll", scheduleBoundsSync, true);
    scrollContainer?.addEventListener("scroll", scheduleBoundsSync);
    scheduleBoundsSync();

    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", scheduleBoundsSync);
      window.removeEventListener("scroll", scheduleBoundsSync, true);
      scrollContainer?.removeEventListener("scroll", scheduleBoundsSync);
      if (syncFrameRef.current !== null) {
        window.cancelAnimationFrame(syncFrameRef.current);
        syncFrameRef.current = null;
      }
      hideNativeBrowser();
    };
  }, [hideNativeBrowser, isVisible, scheduleBoundsSync]);

  useLayoutEffect(() => {
    if (isVisible) {
      scheduleBoundsSync();
    }
  });

  useEffect(() => {
    if (!findOpen) {
      setFindResult(null);
      void desktop().stopFindInBrowserPane(pane.id, "clearSelection");
      return;
    }
    window.requestAnimationFrame(() => {
      findInputRef.current?.focus();
      findInputRef.current?.select();
    });
  }, [findOpen, pane.id]);

  useEffect(() => {
    if (!findOpen) {
      return;
    }
    const timer = window.setTimeout(() => {
      void desktop().findInBrowserPane(pane.id, findValue, { forward: true, findNext: false });
    }, 120);
    return () => {
      window.clearTimeout(timer);
    };
  }, [findOpen, findValue, pane.id]);

  const navigateToInput = async (): Promise<void> => {
    const target = normalizeBrowserNavigationInput(urlValue);
    if (target.kind === "external") {
      await desktop().openExternal(target.url);
      return;
    }
    const state = await desktop().navigateBrowserPane(pane.id, target.url);
    applyBrowserState(state);
  };

  const openExternalBrowser = async (): Promise<void> => {
    const url = browserState?.url || payload.url;
    const target = normalizeBrowserNavigationInput(url);
    await desktop().openExternal(target.url);
  };

  const navigateRelativeFind = (forward: boolean): void => {
    if (!findValue.trim()) {
      return;
    }
    void desktop().findInBrowserPane(pane.id, findValue, { forward, findNext: true });
  };

  return (
    <div
      className="browser-pane"
      onKeyDownCapture={(event) => {
        const ctrlOrMeta = event.ctrlKey || event.metaKey;
        if (!ctrlOrMeta || event.altKey) {
          return;
        }
        const key = event.key.toLowerCase();
        if (key === "l") {
          event.preventDefault();
          inputRef.current?.focus();
          inputRef.current?.select();
          return;
        }
        if (key === "f") {
          event.preventDefault();
          setFindOpen(true);
          window.requestAnimationFrame(() => {
            findInputRef.current?.focus();
            findInputRef.current?.select();
          });
          return;
        }
        if (key === "r") {
          event.preventDefault();
          void desktop().reloadBrowserPane(pane.id).then(applyBrowserState);
          return;
        }
        if (key === "+" || key === "=") {
          event.preventDefault();
          void setZoom(zoomFactor + 0.1);
          return;
        }
        if (key === "-") {
          event.preventDefault();
          void setZoom(zoomFactor - 0.1);
          return;
        }
        if (key === "0") {
          event.preventDefault();
          void setZoom(1);
        }
      }}
    >
      <div className="browser-toolbar browser-toolbar-compact">
        <BrowserIconButton
          icon="back"
          label="Back"
          disabled={!browserState?.canGoBack}
          onClick={() => {
            void desktop().goBackBrowserPane(pane.id).then(applyBrowserState);
          }}
        />
        <BrowserIconButton
          icon="forward"
          label="Forward"
          disabled={!browserState?.canGoForward}
          onClick={() => {
            void desktop().goForwardBrowserPane(pane.id).then(applyBrowserState);
          }}
        />
        <BrowserIconButton
          icon={browserState?.loading ? "stop" : "reload"}
          label={browserState?.loading ? "Stop" : "Reload"}
          onClick={() => {
            void (browserState?.loading
              ? desktop().stopBrowserPane(pane.id)
              : desktop().reloadBrowserPane(pane.id)
            ).then(applyBrowserState);
          }}
        />
        <form
          className="browser-form browser-omnibox"
          onSubmit={(event) => {
            event.preventDefault();
            void navigateToInput().catch((error) => {
              setStatusText(error instanceof Error ? error.message : String(error));
            });
          }}
        >
          <span className={`browser-omnibox-status ${browserState?.loading ? "loading" : ""}`} aria-hidden="true">
            <BrowserIcon icon={browserState?.errorText ? "warning" : "globe"} />
          </span>
          <input
            ref={inputRef}
            className="browser-omnibox-input"
            value={urlValue}
            placeholder="Search or enter address"
            onFocus={(event) => {
              urlFocusedRef.current = true;
              event.currentTarget.select();
            }}
            onBlur={() => {
              urlFocusedRef.current = false;
            }}
            onChange={(event) => setUrlValue(event.target.value)}
          />
        </form>
        <div className="browser-zoom-controls" aria-label="Zoom controls">
          <BrowserIconButton
            icon="minus"
            label="Zoom out"
            onClick={() => {
              void setZoom(zoomFactor - 0.1);
            }}
          />
          <button
            className="browser-zoom-value"
            title="Reset zoom"
            onClick={() => {
              void setZoom(1);
            }}
          >
            {zoomLabel}
          </button>
          <BrowserIconButton
            icon="plus"
            label="Zoom in"
            onClick={() => {
              void setZoom(zoomFactor + 0.1);
            }}
          />
        </div>
        <BrowserIconButton
          icon="find"
          label="Find"
          active={findOpen}
          onClick={() => setFindOpen((current) => !current)}
        />
        <BrowserIconButton
          icon="external"
          label="Open externally"
          onClick={() => {
            void openExternalBrowser();
          }}
        />
        <BrowserIconButton
          icon="devtools"
          label="Open DevTools"
          onClick={() => {
            void desktop().openBrowserDevTools(pane.id);
          }}
        />
      </div>
      {findOpen && (
        <form
          className="browser-findbar"
          onSubmit={(event) => {
            event.preventDefault();
            navigateRelativeFind(true);
          }}
        >
          <BrowserIcon icon="find" />
          <input
            ref={findInputRef}
            value={findValue}
            placeholder="Find in page"
            onChange={(event) => setFindValue(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                setFindOpen(false);
              }
            }}
          />
          <span className="browser-find-count">
            {findResult?.matches
              ? `${findResult.activeMatchOrdinal}/${findResult.matches}`
              : findValue
                ? "0/0"
                : ""}
          </span>
          <BrowserIconButton icon="up" label="Previous match" onClick={() => navigateRelativeFind(false)} />
          <BrowserIconButton icon="down" label="Next match" onClick={() => navigateRelativeFind(true)} />
          <BrowserIconButton icon="close" label="Close find" onClick={() => setFindOpen(false)} />
        </form>
      )}
      {(statusText || download) && (
        <div className="browser-status-strip">
          {statusText && <span className="browser-status-chip error">{statusText}</span>}
          {download && (
            <span className={`browser-status-chip ${download.state}`}>
              {download.state === "completed"
                ? `Downloaded ${download.fileName}`
                : download.state === "cancelled"
                  ? `Cancelled ${download.fileName}`
                  : `Downloading ${download.fileName}`}
            </span>
          )}
        </div>
      )}
      <div
        ref={surfaceRef}
        className="browser-surface"
        onMouseDown={onFocus}
        aria-label="Browser content"
      >
        {!browserState && <div className="browser-surface-loading">Loading browser...</div>}
      </div>
    </div>
  );
}

type BrowserIconName =
  | "back"
  | "forward"
  | "reload"
  | "stop"
  | "globe"
  | "warning"
  | "find"
  | "external"
  | "devtools"
  | "plus"
  | "minus"
  | "up"
  | "down"
  | "close";

function BrowserIconButton({
  icon,
  label,
  active = false,
  disabled = false,
  onClick,
}: {
  icon: BrowserIconName;
  label: string;
  active?: boolean;
  disabled?: boolean;
  onClick: () => void;
}): React.ReactElement {
  return (
    <button
      type="button"
      className={`browser-icon-button ${active ? "active" : ""}`}
      title={label}
      aria-label={label}
      disabled={disabled}
      onClick={onClick}
    >
      <BrowserIcon icon={icon} />
    </button>
  );
}

function BrowserIcon({ icon }: { icon: BrowserIconName }): React.ReactElement {
  const paths: Record<BrowserIconName, React.ReactNode> = {
    back: <path d="M15 5 8 12l7 7M9 12h12" />,
    forward: <path d="m9 5 7 7-7 7M15 12H3" />,
    reload: <path d="M20 12a8 8 0 1 1-2.34-5.66M20 4v6h-6" />,
    stop: <path d="M7 7h10v10H7z" />,
    globe: <><circle cx="12" cy="12" r="9" /><path d="M3 12h18M12 3c2.4 2.6 3.6 5.6 3.6 9s-1.2 6.4-3.6 9c-2.4-2.6-3.6-5.6-3.6-9S9.6 5.6 12 3z" /></>,
    warning: <><path d="m12 4 9 16H3z" /><path d="M12 9v5M12 17h.01" /></>,
    find: <><circle cx="11" cy="11" r="6" /><path d="m16 16 4 4" /></>,
    external: <><path d="M14 4h6v6M20 4l-9 9" /><path d="M20 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h5" /></>,
    devtools: <><path d="m8 9-4 3 4 3M16 9l4 3-4 3M13 5l-2 14" /></>,
    plus: <path d="M12 5v14M5 12h14" />,
    minus: <path d="M5 12h14" />,
    up: <path d="m7 14 5-5 5 5" />,
    down: <path d="m7 10 5 5 5-5" />,
    close: <path d="M6 6l12 12M18 6 6 18" />,
  };

  return (
    <svg
      className="browser-icon"
      viewBox="0 0 24 24"
      aria-hidden="true"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      {paths[icon]}
    </svg>
  );
}

createRoot(document.getElementById("root")!).render(<App />);

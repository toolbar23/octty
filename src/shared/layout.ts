import type {
  BrowserPanePayload,
  ColumnPin,
  NotePanePayload,
  PanePlacement,
  PaneState,
  PaneType,
  SessionState,
  SidebarTarget,
  TerminalKind,
  TerminalPanePayload,
  WorkspaceColumn,
  WorkspaceSnapshot,
} from "./types";
import {
  defaultTerminalCommand,
  normalizeTerminalKind,
  terminalKindLabel,
} from "./terminal-kind";

const DEFAULT_BROWSER_URL = "https://jj-vcs.github.io/jj/latest/";
const SNAPSHOT_LAYOUT_VERSION = 2;
const MIN_COLUMN_WIDTH_PX = 180;
const MAX_COLUMN_WIDTH_PX = 1100;
const DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX = 1800;
const DEFAULT_WIDE_PANE_WIDTH_FRACTION = 0.33;
const DEFAULT_NOTE_PANE_WIDTH_FRACTION = 0.15;
const DEFAULT_BROWSER_PANE_WIDTH_FRACTION = 0.5;

type LegacyPaneLayoutNode = {
  id: string;
  kind: "pane";
  paneId: string;
};

type LegacySplitLayoutNode = {
  id: string;
  kind: "split";
  direction: "row" | "column";
  children: string[];
  sizes: number[];
};

type LegacyStackLayoutNode = {
  id: string;
  kind: "stack";
  children: string[];
  activeChildId: string;
};

type LegacyLayoutNode =
  | LegacyPaneLayoutNode
  | LegacySplitLayoutNode
  | LegacyStackLayoutNode;

type LegacyWorkspaceSnapshot = {
  workspaceId: string;
  rootNodeId: string | null;
  activePaneId: string | null;
  nodes: Record<string, LegacyLayoutNode>;
  panes: Record<string, PaneState>;
  leftSidebarPaneIds: string[];
  rightSidebarPaneIds: string[];
  collapsedPaneIds: string[];
  updatedAt: number;
};

export function createId(prefix: string): string {
  return `${prefix}${globalThis.crypto.randomUUID()}`;
}

function makePaneTitle(type: PaneType, terminalKind: TerminalKind = "shell"): string {
  switch (type) {
    case "shell":
      return terminalKindLabel(terminalKind);
    case "agent-shell":
      return terminalKindLabel("codex");
    case "note":
      return "Note";
    case "browser":
      return "Browser";
    case "diff":
      return "Diff";
  }
}

function makeTerminalPayload(
  kind: TerminalKind,
  workspacePath: string,
  sessionState: SessionState = "missing",
): TerminalPanePayload {
  return {
    kind,
    sessionId: null,
    sessionState,
    cwd: workspacePath,
    command: defaultTerminalCommand(kind),
    exitCode: null,
    autoStart: true,
    restoredBuffer: "",
    embeddedSession: null,
    embeddedSessionCorrelationId: null,
    agentAttentionState: null,
  };
}

function makeBrowserPayload(): BrowserPanePayload {
  return {
    url: DEFAULT_BROWSER_URL,
    title: "Docs",
  };
}

function makeNotePayload(): NotePanePayload {
  return {
    notePath: null,
  };
}

export function createPaneState(
  type: PaneType,
  workspacePath: string,
  terminalKind: TerminalKind = "shell",
): PaneState {
  const id = createId("pane-");
  if (type === "shell" || type === "agent-shell") {
    const normalizedKind = type === "agent-shell" ? "codex" : terminalKind;
    return {
      id,
      type: "shell",
      title: makePaneTitle("shell", normalizedKind),
      payload: makeTerminalPayload(normalizedKind, workspacePath),
    };
  }

  if (type === "note") {
    return {
      id,
      type,
      title: makePaneTitle(type),
      payload: makeNotePayload(),
    };
  }

  if (type === "browser") {
    return {
      id,
      type,
      title: makePaneTitle(type),
      payload: makeBrowserPayload(),
    };
  }

  return {
    id,
    type,
    title: makePaneTitle(type),
    payload: { pinned: false },
  };
}

export function defaultColumnWidthForPane(
  paneOrType: PaneState | PaneType,
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): number {
  const type = typeof paneOrType === "string" ? paneOrType : paneOrType.type;
  const normalizedViewportWidth =
    Number.isFinite(viewportWidth) && viewportWidth > 0
      ? viewportWidth
      : DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX;
  switch (type) {
    case "shell":
    case "agent-shell":
      return clampColumnWidth(normalizedViewportWidth * DEFAULT_WIDE_PANE_WIDTH_FRACTION);
    case "note":
      return clampColumnWidth(normalizedViewportWidth * DEFAULT_NOTE_PANE_WIDTH_FRACTION);
    case "diff":
      return clampColumnWidth(normalizedViewportWidth * DEFAULT_WIDE_PANE_WIDTH_FRACTION);
    case "browser":
      return clampColumnWidth(normalizedViewportWidth * DEFAULT_BROWSER_PANE_WIDTH_FRACTION);
  }
}

function clampColumnWidth(widthPx: number): number {
  return Math.max(MIN_COLUMN_WIDTH_PX, Math.min(MAX_COLUMN_WIDTH_PX, Math.round(widthPx)));
}

function dedupeOrdered<T>(values: T[]): T[] {
  return Array.from(new Set(values));
}

function cloneSnapshot(snapshot: WorkspaceSnapshot): WorkspaceSnapshot {
  return structuredClone(snapshot);
}

function normalizeHeightFractions(
  paneIds: string[],
  heightFractions: number[],
): number[] {
  if (paneIds.length === 0) {
    return [];
  }

  if (heightFractions.length !== paneIds.length) {
    return Array.from({ length: paneIds.length }, () => 1 / paneIds.length);
  }

  const positive = heightFractions.map((value) => (Number.isFinite(value) && value > 0 ? value : 0));
  const total = positive.reduce((sum, value) => sum + value, 0);
  if (total <= 0) {
    return Array.from({ length: paneIds.length }, () => 1 / paneIds.length);
  }

  return positive.map((value) => value / total);
}

function createColumnForPane(
  pane: PaneState,
  pinned: ColumnPin = null,
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): WorkspaceColumn {
  return {
    id: createId("column-"),
    paneIds: [pane.id],
    widthPx: defaultColumnWidthForPane(pane, viewportWidth),
    heightFractions: [1],
    pinned,
  };
}

function removeCenterColumnId(snapshot: WorkspaceSnapshot, columnId: string): void {
  snapshot.centerColumnIds = snapshot.centerColumnIds.filter((id) => id !== columnId);
}

function removeColumnRecord(snapshot: WorkspaceSnapshot, columnId: string): void {
  removeCenterColumnId(snapshot, columnId);
  if (snapshot.pinnedLeftColumnId === columnId) {
    snapshot.pinnedLeftColumnId = null;
  }
  if (snapshot.pinnedRightColumnId === columnId) {
    snapshot.pinnedRightColumnId = null;
  }
  delete snapshot.columns[columnId];
}

function insertCenterColumnId(snapshot: WorkspaceSnapshot, columnId: string, index?: number): void {
  removeCenterColumnId(snapshot, columnId);
  const nextIndex =
    index === undefined
      ? snapshot.centerColumnIds.length
      : Math.max(0, Math.min(index, snapshot.centerColumnIds.length));
  snapshot.centerColumnIds.splice(nextIndex, 0, columnId);
}

function listColumnOrder(snapshot: WorkspaceSnapshot): string[] {
  return dedupeOrdered([
    ...(snapshot.pinnedLeftColumnId ? [snapshot.pinnedLeftColumnId] : []),
    ...snapshot.centerColumnIds,
    ...(snapshot.pinnedRightColumnId ? [snapshot.pinnedRightColumnId] : []),
  ]).filter((columnId) => Boolean(snapshot.columns[columnId]));
}

function listPaneOrder(snapshot: WorkspaceSnapshot): string[] {
  return listColumnOrder(snapshot).flatMap((columnId) =>
    (snapshot.columns[columnId]?.paneIds ?? []).filter((paneId) => Boolean(snapshot.panes[paneId])),
  );
}

function firstAvailablePaneId(snapshot: WorkspaceSnapshot): string | null {
  return listPaneOrder(snapshot)[0] ?? null;
}

export function findPaneColumnId(snapshot: WorkspaceSnapshot, paneId: string): string | null {
  for (const [columnId, column] of Object.entries(snapshot.columns)) {
    if (column.paneIds.includes(paneId)) {
      return columnId;
    }
  }
  return null;
}

function collectPaneIdsFromLegacyNode(
  snapshot: LegacyWorkspaceSnapshot,
  nodeId: string | null,
): string[] {
  if (!nodeId) {
    return [];
  }

  const node = snapshot.nodes[nodeId];
  if (!node) {
    return [];
  }

  if (node.kind === "pane") {
    return snapshot.panes[node.paneId] ? [node.paneId] : [];
  }

  return dedupeOrdered(
    node.children.flatMap((childId) => collectPaneIdsFromLegacyNode(snapshot, childId)),
  );
}

function collectColumnsFromLegacyNode(
  snapshot: LegacyWorkspaceSnapshot,
  nodeId: string | null,
): string[][] {
  if (!nodeId) {
    return [];
  }

  const node = snapshot.nodes[nodeId];
  if (!node) {
    return [];
  }

  if (node.kind === "pane") {
    return snapshot.panes[node.paneId] ? [[node.paneId]] : [];
  }

  if (node.kind === "stack") {
    const paneIds = dedupeOrdered(
      node.children.flatMap((childId) => collectPaneIdsFromLegacyNode(snapshot, childId)),
    );
    return paneIds.length > 0 ? [paneIds] : [];
  }

  if (node.direction === "row") {
    return node.children.flatMap((childId) => collectColumnsFromLegacyNode(snapshot, childId));
  }

  const paneIds = dedupeOrdered(
    node.children.flatMap((childId) => collectPaneIdsFromLegacyNode(snapshot, childId)),
  );
  return paneIds.length > 0 ? [paneIds] : [];
}

function migrateLegacySnapshot(
  legacyValue: LegacyWorkspaceSnapshot,
  workspacePath: string,
): WorkspaceSnapshot {
  const panes = structuredClone(legacyValue.panes ?? {});
  const columns: Record<string, WorkspaceColumn> = {};
  const centerColumnIds: string[] = [];
  const assignedPaneIds = new Set<string>();

  const appendCenterColumn = (paneIds: string[]) => {
    const nextPaneIds = dedupeOrdered(paneIds).filter((paneId) => panes[paneId] && !assignedPaneIds.has(paneId));
    if (nextPaneIds.length === 0) {
      return;
    }
    for (const paneId of nextPaneIds) {
      assignedPaneIds.add(paneId);
    }
    const firstPane = panes[nextPaneIds[0]!]!;
    const columnId = createId("column-");
    columns[columnId] = {
      id: columnId,
      paneIds: nextPaneIds,
      widthPx: clampColumnWidth(defaultColumnWidthForPane(firstPane)),
      heightFractions: normalizeHeightFractions(nextPaneIds, []),
      pinned: null,
    };
    centerColumnIds.push(columnId);
  };

  for (const paneIds of collectColumnsFromLegacyNode(legacyValue, legacyValue.rootNodeId)) {
    appendCenterColumn(paneIds);
  }

  let pinnedLeftColumnId: string | null = null;
  const leftPaneIds = dedupeOrdered(legacyValue.leftSidebarPaneIds ?? []).filter(
    (paneId) => panes[paneId] && !assignedPaneIds.has(paneId),
  );
  if (leftPaneIds.length > 0) {
    for (const paneId of leftPaneIds) {
      assignedPaneIds.add(paneId);
    }
    const columnId = createId("column-");
    columns[columnId] = {
      id: columnId,
      paneIds: leftPaneIds,
      widthPx: clampColumnWidth(defaultColumnWidthForPane(panes[leftPaneIds[0]!]!)),
      heightFractions: normalizeHeightFractions(leftPaneIds, []),
      pinned: "left",
    };
    pinnedLeftColumnId = columnId;
  }

  let pinnedRightColumnId: string | null = null;
  const rightPaneIds = dedupeOrdered(legacyValue.rightSidebarPaneIds ?? []).filter(
    (paneId) => panes[paneId] && !assignedPaneIds.has(paneId),
  );
  if (rightPaneIds.length > 0) {
    for (const paneId of rightPaneIds) {
      assignedPaneIds.add(paneId);
    }
    const columnId = createId("column-");
    columns[columnId] = {
      id: columnId,
      paneIds: rightPaneIds,
      widthPx: clampColumnWidth(defaultColumnWidthForPane(panes[rightPaneIds[0]!]!)),
      heightFractions: normalizeHeightFractions(rightPaneIds, []),
      pinned: "right",
    };
    pinnedRightColumnId = columnId;
  }

  for (const paneId of dedupeOrdered(legacyValue.collapsedPaneIds ?? [])) {
    appendCenterColumn([paneId]);
  }

  for (const pane of Object.values(panes)) {
    if (!assignedPaneIds.has(pane.id)) {
      appendCenterColumn([pane.id]);
    }
  }

  return sanitizeSnapshot(
    {
      layoutVersion: SNAPSHOT_LAYOUT_VERSION,
      workspaceId: legacyValue.workspaceId,
      activePaneId: legacyValue.activePaneId,
      panes,
      columns,
      centerColumnIds,
      pinnedLeftColumnId,
      pinnedRightColumnId,
      updatedAt: legacyValue.updatedAt ?? Date.now(),
    },
    workspacePath,
  );
}

export function setActivePane(
  snapshot: WorkspaceSnapshot,
  paneId: string | null,
): WorkspaceSnapshot {
  const next = cloneSnapshot(snapshot);
  next.activePaneId = paneId;
  next.updatedAt = Date.now();
  return next;
}

export function updatePane(
  snapshot: WorkspaceSnapshot,
  paneId: string,
  updater: (pane: PaneState) => PaneState,
): WorkspaceSnapshot {
  const pane = snapshot.panes[paneId];
  if (!pane) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  next.panes[paneId] = updater(next.panes[paneId]!);
  next.updatedAt = Date.now();
  return next;
}

export function createDefaultSnapshot(
  workspaceId: string,
  workspacePath: string,
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): WorkspaceSnapshot {
  const shellPane = createPaneState("shell", workspacePath);
  const diffPane = createPaneState("diff", workspacePath);
  const shellColumn = createColumnForPane(shellPane, null, viewportWidth);
  const diffColumn = createColumnForPane(diffPane, null, viewportWidth);
  diffColumn.widthPx = defaultColumnWidthForPane(diffPane, viewportWidth);

  return {
    layoutVersion: SNAPSHOT_LAYOUT_VERSION,
    workspaceId,
    activePaneId: shellPane.id,
    panes: {
      [shellPane.id]: shellPane,
      [diffPane.id]: diffPane,
    },
    columns: {
      [shellColumn.id]: shellColumn,
      [diffColumn.id]: diffColumn,
    },
    centerColumnIds: [shellColumn.id, diffColumn.id],
    pinnedLeftColumnId: null,
    pinnedRightColumnId: null,
    updatedAt: Date.now(),
  };
}

export function addPane(
  snapshot: WorkspaceSnapshot,
  type: PaneType,
  workspacePath: string,
  _placement: PanePlacement = "new-column",
  terminalKind: TerminalKind = "shell",
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): WorkspaceSnapshot {
  const next = cloneSnapshot(snapshot);
  const pane = createPaneState(type, workspacePath, terminalKind);
  const column = createColumnForPane(pane, null, viewportWidth);
  next.panes[pane.id] = pane;
  next.columns[column.id] = column;
  next.centerColumnIds.push(column.id);
  next.activePaneId = pane.id;
  next.updatedAt = Date.now();
  return next;
}

export function removePane(snapshot: WorkspaceSnapshot, paneId: string): WorkspaceSnapshot {
  if (!snapshot.panes[paneId]) {
    return snapshot;
  }

  const orderedPaneIds = listPaneOrder(snapshot);
  const removedPaneIndex = orderedPaneIds.indexOf(paneId);
  const nextActivePaneId =
    removedPaneIndex === -1
      ? null
      : orderedPaneIds[removedPaneIndex + 1] ?? orderedPaneIds[removedPaneIndex - 1] ?? null;

  const next = cloneSnapshot(snapshot);
  const columnId = findPaneColumnId(next, paneId);
  delete next.panes[paneId];

  if (columnId) {
    const column = next.columns[columnId];
    column.paneIds = column.paneIds.filter((id) => id !== paneId);
    column.heightFractions = normalizeHeightFractions(column.paneIds, column.heightFractions);
    if (column.paneIds.length === 0) {
      removeColumnRecord(next, columnId);
    }
  }

  if (next.activePaneId === paneId) {
    next.activePaneId =
      (nextActivePaneId && next.panes[nextActivePaneId] ? nextActivePaneId : null) ??
      firstAvailablePaneId(next);
  }

  next.updatedAt = Date.now();
  return next;
}

export function movePaneToColumn(
  snapshot: WorkspaceSnapshot,
  paneId: string,
  targetColumnId: string,
): WorkspaceSnapshot {
  const sourceColumnId = findPaneColumnId(snapshot, paneId);
  if (!sourceColumnId || !snapshot.columns[targetColumnId] || sourceColumnId === targetColumnId) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  const sourceColumn = next.columns[sourceColumnId]!;
  const targetColumn = next.columns[targetColumnId]!;
  sourceColumn.paneIds = sourceColumn.paneIds.filter((id) => id !== paneId);
  sourceColumn.heightFractions = normalizeHeightFractions(sourceColumn.paneIds, sourceColumn.heightFractions);
  targetColumn.paneIds.push(paneId);
  targetColumn.heightFractions = normalizeHeightFractions(targetColumn.paneIds, []);

  if (sourceColumn.paneIds.length === 0) {
    removeColumnRecord(next, sourceColumnId);
  }

  next.activePaneId = paneId;
  next.updatedAt = Date.now();
  return next;
}

export function movePaneToNewColumn(
  snapshot: WorkspaceSnapshot,
  paneId: string,
  targetIndex?: number,
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): WorkspaceSnapshot {
  const sourceColumnId = findPaneColumnId(snapshot, paneId);
  if (!sourceColumnId) {
    return snapshot;
  }

  const sourceColumn = snapshot.columns[sourceColumnId];
  if (!sourceColumn) {
    return snapshot;
  }

  if (sourceColumn.paneIds.length === 1) {
    const currentCenterIndex = snapshot.centerColumnIds.indexOf(sourceColumnId);
    if (sourceColumn.pinned) {
      return moveColumnToCenter(snapshot, sourceColumnId, targetIndex);
    }
    if (targetIndex === undefined || currentCenterIndex === targetIndex) {
      return snapshot;
    }
    return moveColumn(snapshot, sourceColumnId, targetIndex);
  }

  const pane = snapshot.panes[paneId];
  if (!pane) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  const nextSourceColumn = next.columns[sourceColumnId]!;
  nextSourceColumn.paneIds = nextSourceColumn.paneIds.filter((id) => id !== paneId);
  nextSourceColumn.heightFractions = normalizeHeightFractions(
    nextSourceColumn.paneIds,
    nextSourceColumn.heightFractions,
  );

  const newColumnId = createId("column-");
  next.columns[newColumnId] = {
    id: newColumnId,
    paneIds: [paneId],
    widthPx: defaultColumnWidthForPane(pane, viewportWidth),
    heightFractions: [1],
    pinned: null,
  };

  const sourceCenterIndex = next.centerColumnIds.indexOf(sourceColumnId);
  const insertIndex =
    targetIndex !== undefined
      ? targetIndex
      : sourceCenterIndex >= 0
        ? sourceCenterIndex + 1
        : next.centerColumnIds.length;
  insertCenterColumnId(next, newColumnId, insertIndex);
  next.activePaneId = paneId;
  next.updatedAt = Date.now();
  return next;
}

export function moveColumn(
  snapshot: WorkspaceSnapshot,
  columnId: string,
  targetIndex: number,
): WorkspaceSnapshot {
  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  const nextColumn = next.columns[columnId]!;

  if (nextColumn.pinned === "left") {
    next.pinnedLeftColumnId = null;
  } else if (nextColumn.pinned === "right") {
    next.pinnedRightColumnId = null;
  }
  nextColumn.pinned = null;

  insertCenterColumnId(next, columnId, targetIndex);
  next.updatedAt = Date.now();
  return next;
}

export function pinColumn(
  snapshot: WorkspaceSnapshot,
  columnId: string,
  target: SidebarTarget,
): WorkspaceSnapshot {
  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  const nextColumn = next.columns[columnId]!;
  const currentPinnedId = target === "left" ? next.pinnedLeftColumnId : next.pinnedRightColumnId;

  if (currentPinnedId && currentPinnedId !== columnId) {
    const displacedColumn = next.columns[currentPinnedId];
    if (displacedColumn) {
      displacedColumn.pinned = null;
      insertCenterColumnId(next, currentPinnedId, target === "left" ? 0 : next.centerColumnIds.length);
    }
  }

  removeCenterColumnId(next, columnId);
  if (nextColumn.pinned === "left") {
    next.pinnedLeftColumnId = null;
  } else if (nextColumn.pinned === "right") {
    next.pinnedRightColumnId = null;
  }

  nextColumn.pinned = target;
  if (target === "left") {
    next.pinnedLeftColumnId = columnId;
  } else {
    next.pinnedRightColumnId = columnId;
  }
  next.updatedAt = Date.now();
  return next;
}

export function moveColumnToCenter(
  snapshot: WorkspaceSnapshot,
  columnId: string,
  targetIndex?: number,
): WorkspaceSnapshot {
  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  const nextColumn = next.columns[columnId]!;
  const previousPin = nextColumn.pinned;
  if (previousPin === "left") {
    next.pinnedLeftColumnId = null;
  } else if (previousPin === "right") {
    next.pinnedRightColumnId = null;
  }
  nextColumn.pinned = null;
  insertCenterColumnId(
    next,
    columnId,
    targetIndex ?? (previousPin === "left" ? 0 : next.centerColumnIds.length),
  );
  next.updatedAt = Date.now();
  return next;
}

export function setColumnWidth(
  snapshot: WorkspaceSnapshot,
  columnId: string,
  widthPx: number,
): WorkspaceSnapshot {
  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  next.columns[columnId]!.widthPx = clampColumnWidth(widthPx);
  next.updatedAt = Date.now();
  return next;
}

export function resizePaneColumn(
  snapshot: WorkspaceSnapshot,
  paneId: string,
  deltaPx: number,
): WorkspaceSnapshot {
  if (!Number.isFinite(deltaPx) || deltaPx === 0) {
    return snapshot;
  }

  const columnId = findPaneColumnId(snapshot, paneId);
  if (!columnId) {
    return snapshot;
  }

  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  return setColumnWidth(snapshot, columnId, column.widthPx + deltaPx);
}

export function movePaneHorizontally(
  snapshot: WorkspaceSnapshot,
  paneId: string,
  direction: -1 | 1,
  viewportWidth = DEFAULT_LAYOUT_VIEWPORT_WIDTH_PX,
): WorkspaceSnapshot {
  const columnId = findPaneColumnId(snapshot, paneId);
  if (!columnId) {
    return snapshot;
  }

  const column = snapshot.columns[columnId];
  if (!column || column.pinned) {
    return snapshot;
  }

  const centerIndex = snapshot.centerColumnIds.indexOf(columnId);
  if (centerIndex === -1) {
    return snapshot;
  }

  if (column.paneIds.length === 1) {
    const targetIndex = Math.max(
      0,
      Math.min(snapshot.centerColumnIds.length - 1, centerIndex + direction),
    );
    if (targetIndex === centerIndex) {
      return snapshot;
    }
    return moveColumn(snapshot, columnId, targetIndex);
  }

  const targetIndex = direction < 0 ? centerIndex : centerIndex + 1;
  return movePaneToNewColumn(snapshot, paneId, targetIndex, viewportWidth);
}

export function setColumnHeightFractions(
  snapshot: WorkspaceSnapshot,
  columnId: string,
  heightFractions: number[],
): WorkspaceSnapshot {
  const column = snapshot.columns[columnId];
  if (!column) {
    return snapshot;
  }

  const next = cloneSnapshot(snapshot);
  next.columns[columnId]!.heightFractions = normalizeHeightFractions(
    next.columns[columnId]!.paneIds,
    heightFractions,
  );
  next.updatedAt = Date.now();
  return next;
}

export function sanitizeSnapshot(
  snapshot: WorkspaceSnapshot,
  workspacePath: string,
): WorkspaceSnapshot {
  const snapshotLike = snapshot as WorkspaceSnapshot & Partial<LegacyWorkspaceSnapshot>;
  const next =
    snapshotLike.layoutVersion === SNAPSHOT_LAYOUT_VERSION &&
    snapshotLike.columns &&
    snapshotLike.centerColumnIds
      ? cloneSnapshot(snapshotLike as WorkspaceSnapshot)
      : migrateLegacySnapshot(snapshotLike as unknown as LegacyWorkspaceSnapshot, workspacePath);

  next.layoutVersion = SNAPSHOT_LAYOUT_VERSION;

  for (const pane of Object.values(next.panes)) {
    if (pane.type === "shell" || pane.type === "agent-shell") {
      const payload = pane.payload as TerminalPanePayload;
      pane.type = "shell";
      payload.kind = normalizeTerminalKind(payload.kind as string);
      pane.title = terminalKindLabel(payload.kind);
      payload.cwd ||= workspacePath;
      payload.command ||= defaultTerminalCommand(payload.kind);
      payload.sessionState ||= "missing";
      payload.exitCode ??= null;
      payload.autoStart ??= false;
      payload.restoredBuffer ||= "";
      payload.embeddedSession ??= null;
      payload.embeddedSessionCorrelationId ??= null;
      payload.agentAttentionState ??= null;
    }

    if (pane.type === "browser") {
      const payload = pane.payload as BrowserPanePayload;
      payload.url ||= DEFAULT_BROWSER_URL;
      payload.title ||= "Docs";
    }

    if (pane.type === "note") {
      const payload = pane.payload as NotePanePayload;
      payload.notePath ??= null;
    }
  }

  const orderedColumnIds = dedupeOrdered([
    ...next.centerColumnIds,
    ...(next.pinnedLeftColumnId ? [next.pinnedLeftColumnId] : []),
    ...(next.pinnedRightColumnId ? [next.pinnedRightColumnId] : []),
    ...Object.keys(next.columns),
  ]);

  const normalizedColumns: Record<string, WorkspaceColumn> = {};
  const normalizedCenterColumnIds: string[] = [];
  let pinnedLeftColumnId: string | null = null;
  let pinnedRightColumnId: string | null = null;
  const assignedPaneIds = new Set<string>();

  for (const columnId of orderedColumnIds) {
    const column = next.columns[columnId];
    if (!column) {
      continue;
    }

    const paneIds = dedupeOrdered(column.paneIds).filter((paneId) => next.panes[paneId] && !assignedPaneIds.has(paneId));
    if (paneIds.length === 0) {
      continue;
    }

    for (const paneId of paneIds) {
      assignedPaneIds.add(paneId);
    }

    let pinned = column.pinned;
    if (columnId === next.pinnedLeftColumnId) {
      pinned = "left";
    } else if (columnId === next.pinnedRightColumnId) {
      pinned = "right";
    }

    if (pinned === "left") {
      if (pinnedLeftColumnId) {
        pinned = null;
      } else {
        pinnedLeftColumnId = columnId;
      }
    } else if (pinned === "right") {
      if (pinnedRightColumnId) {
        pinned = null;
      } else {
        pinnedRightColumnId = columnId;
      }
    }

    normalizedColumns[columnId] = {
      id: columnId,
      paneIds,
      widthPx: clampColumnWidth(column.widthPx || defaultColumnWidthForPane(next.panes[paneIds[0]!]!)),
      heightFractions: normalizeHeightFractions(paneIds, column.heightFractions ?? []),
      pinned,
    };

    if (!pinned) {
      normalizedCenterColumnIds.push(columnId);
    }
  }

  for (const pane of Object.values(next.panes)) {
    if (assignedPaneIds.has(pane.id)) {
      continue;
    }
    const column = createColumnForPane(pane);
    normalizedColumns[column.id] = column;
    normalizedCenterColumnIds.push(column.id);
  }

  next.columns = normalizedColumns;
  next.centerColumnIds = normalizedCenterColumnIds;
  next.pinnedLeftColumnId = pinnedLeftColumnId;
  next.pinnedRightColumnId = pinnedRightColumnId;

  if (!next.activePaneId || !next.panes[next.activePaneId]) {
    next.activePaneId = firstAvailablePaneId(next);
  } else {
    const activeColumnId = findPaneColumnId(next, next.activePaneId);
    if (!activeColumnId) {
      next.activePaneId = firstAvailablePaneId(next);
    }
  }

  next.updatedAt = Date.now();
  return next;
}

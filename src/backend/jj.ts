import { realpath } from "node:fs/promises";
import { createHash } from "node:crypto";
import {
  encodeMissingWorkspacePath,
  type WorkspaceBookmarkRelation,
  type WorkspaceStatus,
  type WorkspaceState,
} from "../shared/types";
import { summarizeUnifiedDiff } from "../shared/diff-stats";
import { runCheckedCommand } from "./command";

const EFFECTIVE_WORKSPACE_REVSET = "coalesce(@ ~ empty(), @-)";
const DISPLAY_BOOKMARK_REVSET = `heads(first_ancestors(${EFFECTIVE_WORKSPACE_REVSET}) & bookmarks())`;
const CONFLICTED_WORKSPACE_REVSET = `${EFFECTIVE_WORKSPACE_REVSET} & conflicts()`;
const UNPUBLISHED_WORKSPACE_REVSET = "remote_bookmarks()..@ ~ empty()";
const DEFAULT_WORKSPACE_REVSET = "present(default@)";
const NOT_IN_DEFAULT_WORKSPACE_REVSET = "default@..@ ~ empty()";

export const WORKSPACE_STATUS_REVSETS = {
  conflicts: CONFLICTED_WORKSPACE_REVSET,
  unpublished: UNPUBLISHED_WORKSPACE_REVSET,
  defaultWorkspace: DEFAULT_WORKSPACE_REVSET,
  notInDefault: NOT_IN_DEFAULT_WORKSPACE_REVSET,
} as const;

export interface DiscoveredWorkspace {
  id: string;
  workspaceName: string;
  workspacePath: string;
}

interface WorkspaceListEntry {
  workspaceName: string;
  targetChangeId: string | null;
}

const WORKSPACE_LIST_SEPARATOR = "\t";

export function isStaleWorkingCopyError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("The working copy is stale") ||
    message.includes("jj workspace update-stale")
  );
}

async function updateStaleWorkspace(workspacePath: string): Promise<void> {
  await runCheckedCommand(["jj", "workspace", "update-stale"], workspacePath);
}

export async function withStaleWorkspaceRetry<T>(
  workspacePath: string,
  operation: () => Promise<T>,
  updateWorkspace: (workspacePath: string) => Promise<void> = updateStaleWorkspace,
): Promise<T> {
  try {
    return await operation();
  } catch (error) {
    if (!isStaleWorkingCopyError(error)) {
      throw error;
    }
    await updateWorkspace(workspacePath);
    return await operation();
  }
}

function hashWorkspace(rootPath: string, workspaceName: string): string {
  return createHash("sha1")
    .update(`${rootPath}\0${workspaceName}`)
    .digest("hex")
    .slice(0, 16);
}

async function readCurrentWorkspaceName(rootPath: string): Promise<string | null> {
  try {
    const output = await runCheckedCommand([
      "jj",
      "log",
      "-R",
      rootPath,
      "-r",
      "@",
      "-n",
      "1",
      "--no-graph",
      "-T",
      'working_copies.map(|w| w.name()).join(",") ++ "\n"',
    ]);
    const names = output
      .trim()
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean);
    return names[0] ?? null;
  } catch {
    return null;
  }
}

async function readCurrentWorkspaceChangeId(rootPath: string): Promise<string | null> {
  try {
    const output = await runCheckedCommand([
      "jj",
      "log",
      "-R",
      rootPath,
      "-r",
      "@",
      "-n",
      "1",
      "--no-graph",
      "-T",
      'change_id.short() ++ "\n"',
    ]);
    const changeId = output.trim();
    return changeId || null;
  } catch {
    return null;
  }
}

function parseWorkspaceListOutput(output: string): WorkspaceListEntry[] {
  return output
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const [workspaceName, targetChangeId] = line.split(WORKSPACE_LIST_SEPARATOR);
      return {
        workspaceName: workspaceName?.trim() || "",
        targetChangeId: targetChangeId?.trim() || null,
      };
    })
    .filter((entry) => entry.workspaceName.length > 0);
}

export function fallbackWorkspacePath(
  rootPath: string,
  workspaceName: string,
  currentWorkspaceName: string | null,
  currentWorkspaceChangeId: string | null = null,
  workspaceTargetChangeId: string | null = null,
): string {
  if (
    (currentWorkspaceName && workspaceName === currentWorkspaceName) ||
    (currentWorkspaceChangeId && workspaceTargetChangeId === currentWorkspaceChangeId)
  ) {
    return rootPath;
  }
  return encodeMissingWorkspacePath(workspaceName);
}

export async function resolveRepoRoot(inputPath: string): Promise<string> {
  const root = (
    await withStaleWorkspaceRetry(inputPath, () =>
      runCheckedCommand(["jj", "root", "-R", inputPath]),
    )
  ).trim();
  return realpath(root);
}

export async function discoverWorkspaces(rootPath: string): Promise<DiscoveredWorkspace[]> {
  const resolvedRootPath = await realpath(rootPath);
  const { currentWorkspaceName, currentWorkspaceChangeId, listOutput } = await withStaleWorkspaceRetry(
    resolvedRootPath,
    async () => ({
      currentWorkspaceName: await readCurrentWorkspaceName(resolvedRootPath),
      currentWorkspaceChangeId: await readCurrentWorkspaceChangeId(resolvedRootPath),
      listOutput: await runCheckedCommand([
        "jj",
        "workspace",
        "list",
        "-R",
        resolvedRootPath,
        "-T",
        'name ++ "\t" ++ target.change_id().short() ++ "\n"',
      ]),
    }),
  );

  const workspaceEntries = parseWorkspaceListOutput(listOutput);

  const workspaces = await Promise.all(
    workspaceEntries.map(async ({ workspaceName, targetChangeId }) => {
      try {
        const workspacePath = (
          await runCheckedCommand([
            "jj",
            "workspace",
            "root",
            "-R",
            resolvedRootPath,
            "--name",
            workspaceName,
          ])
        ).trim();

        return {
          id: hashWorkspace(resolvedRootPath, workspaceName),
          workspaceName,
          workspacePath: await realpath(workspacePath),
        };
      } catch {
        return {
          id: hashWorkspace(resolvedRootPath, workspaceName),
          workspaceName,
          workspacePath: fallbackWorkspacePath(
            resolvedRootPath,
            workspaceName,
            currentWorkspaceName,
            currentWorkspaceChangeId,
            targetChangeId,
          ),
        };
      }
    }),
  );

  return workspaces;
}

export async function createWorkspace(
  rootPath: string,
  destinationPath: string,
  workspaceName?: string,
): Promise<void> {
  const cmd = ["jj", "workspace", "add", "-R", rootPath];
  if (workspaceName) {
    cmd.push("--name", workspaceName);
  }
  cmd.push(destinationPath);
  await runCheckedCommand(cmd);
}

export async function forgetWorkspace(
  rootPath: string,
  workspaceName: string,
): Promise<void> {
  await runCheckedCommand(["jj", "workspace", "forget", "-R", rootPath, workspaceName]);
}

function parseCount(output: string): number {
  const parsed = Number.parseInt(output.trim(), 10);
  return Number.isFinite(parsed) ? parsed : 0;
}

async function countRevset(workspacePath: string, revset: string): Promise<number> {
  return parseCount(
    await runCheckedCommand(
      ["jj", "log", "-r", revset, "--count"],
      workspacePath,
    ),
  );
}

async function diffStatsForRevset(
  workspacePath: string,
  revset: string,
): Promise<{ addedLines: number; removedLines: number }> {
  const diffText = await runCheckedCommand([
    "jj",
    "diff",
    "-r",
    revset,
    "--git",
    "--color=never",
  ], workspacePath);
  return summarizeUnifiedDiff(diffText);
}

export function classifyWorkspaceState({
  hasConflicts,
  unpublishedChangeCount,
}: {
  hasConflicts: boolean;
  unpublishedChangeCount: number;
}): WorkspaceState {
  if (hasConflicts) {
    return "conflicted";
  }
  if (unpublishedChangeCount === 0) {
    return "published";
  }
  return "draft";
}

function parseBookmarks(output: string): string[] {
  return output
    .trim()
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
}

function classifyBookmarkRelation({
  exactBookmarks,
  displayBookmarks,
}: {
  exactBookmarks: string[];
  displayBookmarks: string[];
}): WorkspaceBookmarkRelation {
  if (exactBookmarks.length > 0) {
    return "exact";
  }
  if (displayBookmarks.length > 0) {
    return "above";
  }
  return "none";
}

export async function readWorkspaceStatus(workspacePath: string): Promise<WorkspaceStatus> {
  const readStatus = () =>
    Promise.all([
      runCheckedCommand([
        "jj",
        "log",
        "-r",
        EFFECTIVE_WORKSPACE_REVSET,
        "-n",
        "1",
        "--no-graph",
        "-T",
        "bookmarks.map(|b| b.name()).join(\",\") ++ \"\\n\"",
      ], workspacePath),
      runCheckedCommand([
        "jj",
        "log",
        "-r",
        DISPLAY_BOOKMARK_REVSET,
        "-n",
        "1",
        "--no-graph",
        "-T",
        "bookmarks.map(|b| b.name()).join(\",\") ++ \"\\n\"",
      ], workspacePath),
      runCheckedCommand(["jj", "diff", "-r", "@", "--git", "--color=never"], workspacePath),
      runCheckedCommand([
        "jj",
        "diff",
        "-r",
        EFFECTIVE_WORKSPACE_REVSET,
        "--git",
        "--color=never",
      ], workspacePath),
      countRevset(workspacePath, CONFLICTED_WORKSPACE_REVSET),
      countRevset(workspacePath, UNPUBLISHED_WORKSPACE_REVSET),
      diffStatsForRevset(workspacePath, UNPUBLISHED_WORKSPACE_REVSET),
      countRevset(workspacePath, DEFAULT_WORKSPACE_REVSET),
    ]);

  const [
    exactBookmarkOutput,
    displayBookmarkOutput,
    diffText,
    effectiveDiffText,
    conflictedCount,
    unpublishedChangeCount,
    unpublishedDiffStats,
    defaultWorkspaceCount,
  ] = await withStaleWorkspaceRetry(workspacePath, readStatus);
  const notInDefaultAvailable = defaultWorkspaceCount > 0;
  const [notInDefaultChangeCount, notInDefaultDiffStats] = notInDefaultAvailable
    ? await withStaleWorkspaceRetry(workspacePath, () =>
        Promise.all([
          countRevset(workspacePath, NOT_IN_DEFAULT_WORKSPACE_REVSET),
          diffStatsForRevset(workspacePath, NOT_IN_DEFAULT_WORKSPACE_REVSET),
        ]),
      )
    : [0, { addedLines: 0, removedLines: 0 }];

  const exactBookmarks = parseBookmarks(exactBookmarkOutput);
  const displayBookmarks = parseBookmarks(displayBookmarkOutput);
  const bookmarks = exactBookmarks.length > 0 ? exactBookmarks : displayBookmarks;
  const bookmarkRelation = classifyBookmarkRelation({
    exactBookmarks,
    displayBookmarks,
  });
  const hasWorkingCopyChanges = diffText.trim().length > 0;
  const hasConflicts = conflictedCount > 0;
  const workspaceState = classifyWorkspaceState({
    hasConflicts,
    unpublishedChangeCount,
  });
  const effectiveDiffStats = summarizeUnifiedDiff(effectiveDiffText);

  return {
    workspaceState,
    hasWorkingCopyChanges,
    effectiveAddedLines: effectiveDiffStats.addedLines,
    effectiveRemovedLines: effectiveDiffStats.removedLines,
    hasConflicts,
    unpublishedChangeCount,
    unpublishedAddedLines: unpublishedDiffStats.addedLines,
    unpublishedRemovedLines: unpublishedDiffStats.removedLines,
    notInDefaultAvailable,
    notInDefaultChangeCount,
    notInDefaultAddedLines: notInDefaultDiffStats.addedLines,
    notInDefaultRemovedLines: notInDefaultDiffStats.removedLines,
    bookmarks,
    bookmarkRelation,
    unreadNotes: 0,
    activeAgentCount: 0,
    agentAttentionState: null,
    recentActivityAt: Date.now(),
    diffText,
  };
}

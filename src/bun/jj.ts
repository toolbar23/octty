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
const PUBLISHED_WORKSPACE_REVSET = `${EFFECTIVE_WORKSPACE_REVSET} & ::remote_bookmarks()`;
const MERGED_LOCAL_WORKSPACE_REVSET = `${EFFECTIVE_WORKSPACE_REVSET} & ::(working_copies() ~ @)`;
const CONFLICTED_WORKSPACE_REVSET = `${EFFECTIVE_WORKSPACE_REVSET} & conflicts()`;

export interface DiscoveredWorkspace {
  id: string;
  workspaceName: string;
  workspacePath: string;
}

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

export function fallbackWorkspacePath(
  rootPath: string,
  workspaceName: string,
  currentWorkspaceName: string | null,
): string {
  if (currentWorkspaceName && workspaceName === currentWorkspaceName) {
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
  const { currentWorkspaceName, namesOutput } = await withStaleWorkspaceRetry(
    resolvedRootPath,
    async () => ({
      currentWorkspaceName: await readCurrentWorkspaceName(resolvedRootPath),
      namesOutput: await runCheckedCommand([
        "jj",
        "workspace",
        "list",
        "-R",
        resolvedRootPath,
        "-T",
        "name ++ \"\\n\"",
      ]),
    }),
  );

  const names = namesOutput
    .split("\n")
    .map((name) => name.trim())
    .filter(Boolean);

  const workspaces = await Promise.all(
    names.map(async (workspaceName) => {
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

export function classifyWorkspaceState({
  hasConflicts,
  isPublished,
  isMergedLocal,
}: {
  hasConflicts: boolean;
  isPublished: boolean;
  isMergedLocal: boolean;
}): WorkspaceState {
  if (hasConflicts) {
    return "conflicted";
  }
  if (isPublished) {
    return "published";
  }
  if (isMergedLocal) {
    return "merged-local";
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
      countRevset(workspacePath, PUBLISHED_WORKSPACE_REVSET),
      countRevset(workspacePath, MERGED_LOCAL_WORKSPACE_REVSET),
    ]);

  const [
    exactBookmarkOutput,
    displayBookmarkOutput,
    diffText,
    effectiveDiffText,
    conflictedCount,
    publishedCount,
    mergedLocalCount,
  ] = await withStaleWorkspaceRetry(workspacePath, readStatus);

  const exactBookmarks = parseBookmarks(exactBookmarkOutput);
  const displayBookmarks = parseBookmarks(displayBookmarkOutput);
  const bookmarks = exactBookmarks.length > 0 ? exactBookmarks : displayBookmarks;
  const bookmarkRelation = classifyBookmarkRelation({
    exactBookmarks,
    displayBookmarks,
  });
  const hasWorkingCopyChanges = diffText.trim().length > 0;
  const workspaceState = classifyWorkspaceState({
    hasConflicts: conflictedCount > 0,
    isPublished: publishedCount > 0,
    isMergedLocal: mergedLocalCount > 0,
  });
  const effectiveDiffStats = summarizeUnifiedDiff(effectiveDiffText);

  return {
    workspaceState,
    hasWorkingCopyChanges,
    effectiveAddedLines: effectiveDiffStats.addedLines,
    effectiveRemovedLines: effectiveDiffStats.removedLines,
    bookmarks,
    bookmarkRelation,
    unreadNotes: 0,
    activeAgentCount: 0,
    agentAttentionState: null,
    recentActivityAt: Date.now(),
    diffText,
  };
}

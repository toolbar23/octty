import { realpath } from "node:fs/promises";
import { createHash } from "node:crypto";
import {
  encodeMissingWorkspacePath,
  type WorkspaceStatus,
} from "../shared/types";
import { runCheckedCommand } from "./command";

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

export async function readWorkspaceStatus(workspacePath: string): Promise<WorkspaceStatus> {
  const readStatus = () =>
    Promise.all([
      runCheckedCommand([
        "jj",
        "log",
        "-r",
        "@",
        "-n",
        "1",
        "--no-graph",
        "-T",
        "bookmarks.map(|b| b.name()).join(\",\") ++ \"\\n\"",
      ], workspacePath),
      runCheckedCommand(["jj", "diff", "-r", "@", "--git", "--color=never"], workspacePath),
    ]);

  const [bookmarkOutput, diffText] = await withStaleWorkspaceRetry(workspacePath, readStatus);

  const bookmarks = bookmarkOutput
    .trim()
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);

  return {
    dirty: diffText.trim().length > 0,
    bookmarks,
    unreadNotes: 0,
    activeAgentCount: 0,
    agentAttentionState: null,
    recentActivityAt: Date.now(),
    diffText,
  };
}

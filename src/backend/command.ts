import { spawn } from "node:child_process";
import { sanitizeChildEnv } from "./env";

export interface CommandResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export async function runCommand(
  cmd: string[],
  cwd?: string,
): Promise<CommandResult> {
  const proc = spawn(cmd[0]!, cmd.slice(1), {
    cwd,
    env: sanitizeChildEnv(),
    stdio: ["ignore", "pipe", "pipe"],
  });

  const stdoutChunks: Buffer[] = [];
  const stderrChunks: Buffer[] = [];
  proc.stdout?.on("data", (chunk: Buffer | string) => {
    stdoutChunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  });
  proc.stderr?.on("data", (chunk: Buffer | string) => {
    stderrChunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  });

  const exitCode = await new Promise<number>((resolve, reject) => {
    proc.once("error", reject);
    proc.once("close", (code) => resolve(code ?? 0));
  });

  return {
    stdout: Buffer.concat(stdoutChunks).toString("utf8"),
    stderr: Buffer.concat(stderrChunks).toString("utf8"),
    exitCode,
  };
}

export async function runCheckedCommand(
  cmd: string[],
  cwd?: string,
): Promise<string> {
  const result = await runCommand(cmd, cwd);
  if (result.exitCode !== 0) {
    throw new Error(result.stderr.trim() || result.stdout.trim() || `Command failed: ${cmd.join(" ")}`);
  }

  return result.stdout;
}

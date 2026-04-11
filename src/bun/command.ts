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
  const proc = Bun.spawn({
    cmd,
    cwd,
    stdout: "pipe",
    stderr: "pipe",
    env: sanitizeChildEnv(),
  });

  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);

  return {
    stdout,
    stderr,
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

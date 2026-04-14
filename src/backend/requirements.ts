import { runCommand } from "./command";

type RuntimeDependency = {
  command: string;
  args: string[];
  label: string;
};

export const RUNTIME_DEPENDENCIES: RuntimeDependency[] = [
  {
    command: "jj",
    args: ["--version"],
    label: "jj",
  },
  {
    command: "tmux",
    args: ["-V"],
    label: "tmux",
  },
];

export async function assertRuntimeDependencies(
  checkCommand: (command: string, args: string[]) => Promise<void> = async (command, args) => {
    const result = await runCommand([command, ...args]);
    if (result.exitCode !== 0) {
      throw new Error(result.stderr.trim() || result.stdout.trim() || `${command} exited with code ${result.exitCode}`);
    }
  },
): Promise<void> {
  const missing: string[] = [];
  const failures: string[] = [];

  for (const dependency of RUNTIME_DEPENDENCIES) {
    try {
      await checkCommand(dependency.command, dependency.args);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (
        error &&
        typeof error === "object" &&
        "code" in error &&
        (error as NodeJS.ErrnoException).code === "ENOENT"
      ) {
        missing.push(dependency.label);
        continue;
      }
      failures.push(`${dependency.label}: ${message}`);
    }
  }

  if (missing.length === 0 && failures.length === 0) {
    return;
  }

  const lines = ["Octty requires these tools to be available on PATH:"];
  if (missing.length > 0) {
    lines.push(`Missing: ${missing.join(", ")}`);
  }
  if (failures.length > 0) {
    lines.push(`Failed checks: ${failures.join("; ")}`);
  }
  throw new Error(lines.join("\n"));
}

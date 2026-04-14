import { describe, expect, test, vi } from "vitest";
import { assertRuntimeDependencies } from "./requirements";

describe("assertRuntimeDependencies", () => {
  test("passes when all runtime commands are available", async () => {
    const checkCommand = vi.fn(async () => {});
    await expect(assertRuntimeDependencies(checkCommand)).resolves.toBeUndefined();
    expect(checkCommand).toHaveBeenCalledTimes(2);
  });

  test("reports missing commands clearly", async () => {
    const checkCommand = vi.fn(async (command: string) => {
      if (command === "tmux") {
        const error = new Error("spawn tmux ENOENT") as NodeJS.ErrnoException;
        error.code = "ENOENT";
        throw error;
      }
    });

    await expect(assertRuntimeDependencies(checkCommand)).rejects.toThrow(
      "Missing: tmux",
    );
  });
});

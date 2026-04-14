import { describe, expect, test, vi } from "vitest";

describe("resolveStateDbPath", () => {
  test("prefers the Electron user data directory when no legacy database exists", async () => {
    vi.resetModules();
    const existsSync = vi.fn(() => false);
    vi.doMock("node:fs", () => ({ existsSync }));

    const { resolveStateDbPath } = await import("./app-paths");
    expect(
      resolveStateDbPath({
        OCTTY_USER_DATA_PATH: "/appdata/Octty",
      }),
    ).toBe("/appdata/Octty/state.sqlite");
  });

  test("falls back to the legacy database when a migrated database is not present yet", async () => {
    vi.resetModules();
    const existsSync = vi.fn((targetPath: string) => targetPath === "/home/pm/.local/share/octty/state.sqlite");
    vi.doMock("node:fs", () => ({ existsSync }));

    const { resolveStateDbPath } = await import("./app-paths");
    expect(
      resolveStateDbPath({
        OCTTY_USER_DATA_PATH: "/appdata/Octty",
      }),
    ).toBe("/home/pm/.local/share/octty/state.sqlite");
  });
});

import { describe, expect, test } from "vitest";
import { sanitizeChildEnv } from "./env";

describe("sanitizeChildEnv", () => {
  test("drops loader-specific variables and keeps normal shell environment", () => {
    const env = sanitizeChildEnv({
      HOME: "/home/pm",
      PATH: "/usr/bin:/bin",
      SHELL: "/bin/bash",
      LD_PRELOAD: "./libcef.so:./libvk_swiftshader.so",
      LD_LIBRARY_PATH: "/tmp/cef",
      DYLD_INSERT_LIBRARIES: "/tmp/lib.dylib",
      TMUX: "/tmp/tmux-1000/default,1234,0",
      TMUX_PANE: "%1",
    });

    expect(env).toEqual({
      HOME: "/home/pm",
      PATH: "/usr/bin:/bin",
      SHELL: "/bin/bash",
    });
  });
});

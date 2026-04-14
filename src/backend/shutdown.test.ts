import { describe, expect, test } from "vitest";
import { createShutdownController } from "./shutdown";

describe("createShutdownController", () => {
  test("runs cleanup once and exits once", () => {
    const calls: string[] = [];
    const controller = createShutdownController({
      unregisterShortcuts: () => calls.push("shortcuts"),
      stopServer: () => calls.push("server"),
      disposeService: () => calls.push("service"),
      exit: () => {
        calls.push("exit");
      },
    });

    controller.shutdown();
    controller.shutdown();

    expect(calls).toEqual(["shortcuts", "server", "service", "exit"]);
    expect(controller.isShuttingDown()).toBe(true);
  });

  test("can clean up without exiting immediately", () => {
    const calls: string[] = [];
    const controller = createShutdownController({
      unregisterShortcuts: () => calls.push("shortcuts"),
      stopServer: () => calls.push("server"),
      disposeService: () => calls.push("service"),
      exit: () => {
        calls.push("exit");
      },
    });

    controller.shutdown(false);

    expect(calls).toEqual(["shortcuts", "server", "service"]);
  });
});

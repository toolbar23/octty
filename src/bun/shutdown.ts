type ShutdownStep = () => void;

type ShutdownControllerOptions = {
  unregisterShortcuts: ShutdownStep;
  stopServer: ShutdownStep;
  disposeService: ShutdownStep;
  exit: (code?: number) => void;
};

export function createShutdownController(options: ShutdownControllerOptions): {
  shutdown: (exitProcess?: boolean) => void;
  isShuttingDown: () => boolean;
} {
  let shuttingDown = false;

  const runStep = (step: ShutdownStep, label: string): void => {
    try {
      step();
    } catch (error) {
      console.error(`[shutdown] ${label} failed`, error);
    }
  };

  return {
    shutdown(exitProcess = true): void {
      if (shuttingDown) {
        return;
      }
      shuttingDown = true;
      runStep(options.unregisterShortcuts, "unregisterShortcuts");
      runStep(options.stopServer, "stopServer");
      runStep(options.disposeService, "disposeService");
      if (exitProcess) {
        options.exit(0);
      }
    },
    isShuttingDown(): boolean {
      return shuttingDown;
    },
  };
}

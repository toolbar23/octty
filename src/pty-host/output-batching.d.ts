declare module "./output-batching.mjs" {
  export interface OutputMessage {
    type: "output";
    sessionId: string;
    data: string;
  }

  export interface OutputBatcher {
    add: (sessionId: string, data: string) => void;
    flush: (sessionId: string) => void;
    clear: (sessionId: string) => void;
  }

  export interface OutputBatcherOptions<THandle = unknown> {
    send: (message: OutputMessage) => void;
    flushDelayMs?: number;
    maxBatchSize?: number;
    schedule?: (callback: () => void, delayMs: number) => THandle;
    cancel?: (handle: THandle) => void;
  }

  export function createOutputBatcher<THandle = unknown>(
    options: OutputBatcherOptions<THandle>,
  ): OutputBatcher;
}

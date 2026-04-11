export interface TimerScheduler<THandle = unknown> {
  schedule: (callback: () => void, delayMs: number) => THandle;
  cancel: (handle: THandle) => void;
}

export interface BufferedStringFlusherOptions<THandle> {
  flushDelayMs: number;
  maxBatchSize: number;
  onFlush: (data: string) => void;
  scheduler: TimerScheduler<THandle>;
  measureSize?: (data: string) => number;
  shouldFlushImmediately?: (data: string) => boolean;
}

const textEncoder = new TextEncoder();

export function measureTerminalBytes(data: string): number {
  return textEncoder.encode(data).length;
}

export function shouldFlushTerminalInputImmediately(data: string): boolean {
  return data.includes("\r") || data.includes("\n");
}

export function takeStringChunk(queue: string[], maxChunkSize: number): string {
  if (maxChunkSize <= 0 || queue.length === 0) {
    return "";
  }

  let chunk = "";
  let remaining = maxChunkSize;

  while (remaining > 0 && queue.length > 0) {
    const next = queue[0]!;
    if (next.length <= remaining) {
      chunk += next;
      remaining -= next.length;
      queue.shift();
      continue;
    }

    chunk += next.slice(0, remaining);
    queue[0] = next.slice(remaining);
    remaining = 0;
  }

  return chunk;
}

export class BufferedStringFlusher<THandle = unknown> {
  private buffer = "";
  private bufferSize = 0;
  private timer: THandle | null = null;
  private readonly measureSize: (data: string) => number;

  constructor(private readonly options: BufferedStringFlusherOptions<THandle>) {
    this.measureSize = options.measureSize ?? measureTerminalBytes;
  }

  add(data: string): void {
    if (!data) {
      return;
    }

    this.buffer += data;
    this.bufferSize += this.measureSize(data);

    if (
      this.bufferSize >= this.options.maxBatchSize ||
      this.options.shouldFlushImmediately?.(data)
    ) {
      this.flush();
      return;
    }

    if (this.timer !== null) {
      return;
    }

    this.timer = this.options.scheduler.schedule(() => {
      this.timer = null;
      this.flush();
    }, this.options.flushDelayMs);
  }

  flush(): void {
    if (!this.buffer) {
      this.clearTimer();
      return;
    }

    const chunk = this.buffer;
    this.buffer = "";
    this.bufferSize = 0;
    this.clearTimer();
    this.options.onFlush(chunk);
  }

  dispose(): void {
    this.flush();
  }

  hasPendingData(): boolean {
    return this.buffer.length > 0;
  }

  private clearTimer(): void {
    if (this.timer === null) {
      return;
    }

    this.options.scheduler.cancel(this.timer);
    this.timer = null;
  }
}

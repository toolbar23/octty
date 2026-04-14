import { describe, expect, test } from "vitest";
import {
  BufferedStringFlusher,
  shouldFlushTerminalInputImmediately,
  takeStringChunk,
} from "./terminal-batching";

interface FakeTimer {
  id: number;
}

function createFakeScheduler() {
  let nextId = 1;
  const callbacks = new Map<number, () => void>();

  return {
    schedule(callback: () => void): FakeTimer {
      const handle = { id: nextId += 1 };
      callbacks.set(handle.id, callback);
      return handle;
    },
    cancel(handle: FakeTimer): void {
      callbacks.delete(handle.id);
    },
    flushAll(): void {
      const pending = Array.from(callbacks.values());
      callbacks.clear();
      for (const callback of pending) {
        callback();
      }
    },
    pendingCount(): number {
      return callbacks.size;
    },
  };
}

describe("BufferedStringFlusher", () => {
  test("coalesces printable input until the timer fires", () => {
    const scheduler = createFakeScheduler();
    const flushed: string[] = [];
    const flusher = new BufferedStringFlusher<FakeTimer>({
      flushDelayMs: 4,
      maxBatchSize: 256,
      scheduler,
      measureSize: (data) => data.length,
      shouldFlushImmediately: shouldFlushTerminalInputImmediately,
      onFlush: (data) => {
        flushed.push(data);
      },
    });

    flusher.add("a");
    flusher.add("b");
    flusher.add("c");

    expect(flushed).toEqual([]);
    expect(scheduler.pendingCount()).toBe(1);

    scheduler.flushAll();

    expect(flushed).toEqual(["abc"]);
    expect(flusher.hasPendingData()).toBe(false);
  });

  test("flushes immediately on newline input", () => {
    const scheduler = createFakeScheduler();
    const flushed: string[] = [];
    const flusher = new BufferedStringFlusher<FakeTimer>({
      flushDelayMs: 4,
      maxBatchSize: 256,
      scheduler,
      measureSize: (data) => data.length,
      shouldFlushImmediately: shouldFlushTerminalInputImmediately,
      onFlush: (data) => {
        flushed.push(data);
      },
    });

    flusher.add("echo hello");
    flusher.add("\r");

    expect(flushed).toEqual(["echo hello\r"]);
    expect(scheduler.pendingCount()).toBe(0);
  });

  test("flushes when the batch size threshold is reached", () => {
    const scheduler = createFakeScheduler();
    const flushed: string[] = [];
    const flusher = new BufferedStringFlusher<FakeTimer>({
      flushDelayMs: 4,
      maxBatchSize: 5,
      scheduler,
      measureSize: (data) => data.length,
      onFlush: (data) => {
        flushed.push(data);
      },
    });

    flusher.add("abc");
    flusher.add("de");

    expect(flushed).toEqual(["abcde"]);
    expect(scheduler.pendingCount()).toBe(0);
  });

  test("dispose flushes pending data", () => {
    const scheduler = createFakeScheduler();
    const flushed: string[] = [];
    const flusher = new BufferedStringFlusher<FakeTimer>({
      flushDelayMs: 4,
      maxBatchSize: 256,
      scheduler,
      measureSize: (data) => data.length,
      onFlush: (data) => {
        flushed.push(data);
      },
    });

    flusher.add("pending");
    flusher.dispose();

    expect(flushed).toEqual(["pending"]);
    expect(scheduler.pendingCount()).toBe(0);
  });
});

describe("takeStringChunk", () => {
  test("preserves order while splitting oversized output", () => {
    const queue = ["hello", "world", "!"];

    expect(takeStringChunk(queue, 7)).toBe("hellowo");
    expect(queue).toEqual(["rld", "!"]);
    expect(takeStringChunk(queue, 7)).toBe("rld!");
    expect(queue).toEqual([]);
  });
});

import { describe, expect, test } from "vitest";
import { createOutputBatcher } from "./output-batching.mjs";

function createFakeScheduler() {
  let nextId = 0;
  const callbacks = new Map();

  return {
    schedule(callback) {
      nextId += 1;
      callbacks.set(nextId, callback);
      return { id: nextId };
    },
    cancel(handle) {
      callbacks.delete(handle.id);
    },
    flushAll() {
      const pending = Array.from(callbacks.values());
      callbacks.clear();
      for (const callback of pending) {
        callback();
      }
    },
    pendingCount() {
      return callbacks.size;
    },
  };
}

describe("createOutputBatcher", () => {
  test("coalesces output chunks until the timer fires", () => {
    const scheduler = createFakeScheduler();
    const sent = [];
    const batcher = createOutputBatcher({
      send(message) {
        sent.push(message);
      },
      flushDelayMs: 4,
      maxBatchSize: 16_384,
      schedule: scheduler.schedule,
      cancel: scheduler.cancel,
    });

    batcher.add("session-1", "hel");
    batcher.add("session-1", "lo");

    expect(sent).toEqual([]);
    expect(scheduler.pendingCount()).toBe(1);

    scheduler.flushAll();

    expect(sent).toEqual([
      { type: "output", sessionId: "session-1", data: "hello" },
    ]);
  });

  test("flushes immediately once the byte threshold is crossed", () => {
    const scheduler = createFakeScheduler();
    const sent = [];
    const batcher = createOutputBatcher({
      send(message) {
        sent.push(message);
      },
      flushDelayMs: 4,
      maxBatchSize: 5,
      schedule: scheduler.schedule,
      cancel: scheduler.cancel,
    });

    batcher.add("session-1", "abc");
    batcher.add("session-1", "de");

    expect(sent).toEqual([
      { type: "output", sessionId: "session-1", data: "abcde" },
    ]);
    expect(scheduler.pendingCount()).toBe(0);
  });

  test("flushes pending output before exit", () => {
    const scheduler = createFakeScheduler();
    const sent = [];
    const batcher = createOutputBatcher({
      send(message) {
        sent.push(message);
      },
      flushDelayMs: 4,
      maxBatchSize: 16_384,
      schedule: scheduler.schedule,
      cancel: scheduler.cancel,
    });

    batcher.add("session-1", "pending");
    batcher.flush("session-1");

    expect(sent).toEqual([
      { type: "output", sessionId: "session-1", data: "pending" },
    ]);
    expect(scheduler.pendingCount()).toBe(0);
  });
});

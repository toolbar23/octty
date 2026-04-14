const DEFAULT_OUTPUT_BATCH_DELAY_MS = 4;
const DEFAULT_OUTPUT_BATCH_SIZE = 16 * 1024;

export function createOutputBatcher({
  send,
  flushDelayMs = DEFAULT_OUTPUT_BATCH_DELAY_MS,
  maxBatchSize = DEFAULT_OUTPUT_BATCH_SIZE,
  schedule = setTimeout,
  cancel = clearTimeout,
}) {
  const buffers = new Map();

  function flush(sessionId) {
    const pending = buffers.get(sessionId);
    if (!pending) {
      return;
    }

    if (pending.timer) {
      cancel(pending.timer);
      pending.timer = null;
    }

    if (!pending.data) {
      buffers.delete(sessionId);
      return;
    }

    send({
      type: "output",
      sessionId,
      data: pending.data,
    });
    buffers.delete(sessionId);
  }

  function add(sessionId, data) {
    if (!data) {
      return;
    }

    let pending = buffers.get(sessionId);
    if (!pending) {
      pending = { data: "", size: 0, timer: null };
      buffers.set(sessionId, pending);
    }

    pending.data += data;
    pending.size += Buffer.byteLength(data);
    if (pending.size >= maxBatchSize) {
      flush(sessionId);
      return;
    }

    if (!pending.timer) {
      pending.timer = schedule(() => {
        pending.timer = null;
        flush(sessionId);
      }, flushDelayMs);
    }
  }

  function clear(sessionId) {
    const pending = buffers.get(sessionId);
    if (!pending) {
      return;
    }
    if (pending.timer) {
      cancel(pending.timer);
    }
    buffers.delete(sessionId);
  }

  return {
    add,
    flush,
    clear,
  };
}

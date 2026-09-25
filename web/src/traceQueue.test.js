import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createTraceQueue } from "./traceQueue.js";

/** A Promise whose settlement we control, to model an in-flight trace. */
function deferred() {
  let resolve, reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

describe("createTraceQueue", () => {
  it("runs an interactive task after the debounce window", async () => {
    const d = deferred();
    const execute = vi.fn(() => d.promise);
    const onBusyChange = vi.fn();
    const q = createTraceQueue({ execute, onBusyChange });

    q.enqueueInteractive("vectorize", { a: 1 });
    expect(execute).not.toHaveBeenCalled(); // debounced

    vi.advanceTimersByTime(300);
    expect(execute).toHaveBeenCalledTimes(1);
    const task = execute.mock.calls[0][0];
    expect(task.kind).toBe("interactive");
    expect(task.stage).toBe("vectorize");
    expect(task.params).toEqual({ a: 1 });
    // Busy flag scoped to the stage.
    expect(onBusyChange).toHaveBeenNthCalledWith(1, "vectorize");

    d.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(onBusyChange).toHaveBeenNthCalledWith(2, null);
  });

  it("merges rapid interactive calls into the last one", async () => {
    const d = deferred();
    const execute = vi.fn(() => d.promise);
    const q = createTraceQueue({ execute, onBusyChange: vi.fn() });

    q.enqueueInteractive("vectorize", { a: 1 });
    vi.advanceTimersByTime(100);
    q.enqueueInteractive("vectorize", { a: 2 });
    vi.advanceTimersByTime(100);
    q.enqueueInteractive("vectorize", { a: 3 });
    vi.advanceTimersByTime(300);

    expect(execute).toHaveBeenCalledTimes(1);
    expect(execute.mock.calls[0][0].params).toEqual({ a: 3 });
    d.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  it("runs a background task immediately when idle and never fires busy", async () => {
    const d = deferred();
    const execute = vi.fn(() => d.promise);
    const onBusyChange = vi.fn();
    const q = createTraceQueue({ execute, onBusyChange });

    q.enqueueBackground({ a: 1 });
    expect(execute).toHaveBeenCalledTimes(1);
    expect(execute.mock.calls[0][0].kind).toBe("background");
    expect(execute.mock.calls[0][0].stage).toBe("chain");
    expect(onBusyChange).not.toHaveBeenCalled();

    d.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  it("dedupes a background task already in flight with the same params", async () => {
    const d = deferred();
    const execute = vi.fn(() => d.promise);
    const q = createTraceQueue({ execute, onBusyChange: vi.fn() });

    q.enqueueBackground({ a: 1 });
    q.enqueueBackground({ a: 1 }); // same params, in flight -> dropped
    expect(execute).toHaveBeenCalledTimes(1);

    d.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  it("queues a second background task until the first finishes", async () => {
    const d1 = deferred();
    const d2 = deferred();
    const execute = vi.fn((task) => (task.params.a === 1 ? d1.promise : d2.promise));
    const q = createTraceQueue({ execute, onBusyChange: vi.fn() });

    q.enqueueBackground({ a: 1 });
    q.enqueueBackground({ a: 2 });
    expect(execute).toHaveBeenCalledTimes(1); // only the first runs now

    d1.resolve();
    await vi.advanceTimersByTimeAsync(0);
    expect(execute).toHaveBeenCalledTimes(2); // second runs after the first

    d2.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  it("drops queued background work when the user interacts", async () => {
    const d1 = deferred();
    const d2 = deferred();
    const execute = vi.fn((task) =>
      task.kind === "background" ? (task.params.a === 1 ? d1.promise : d2.promise) : deferred().promise
    );
    const q = createTraceQueue({ execute, onBusyChange: vi.fn() });

    q.enqueueBackground({ a: 1 }); // in flight
    q.enqueueBackground({ a: 2 }); // queued behind it
    expect(execute).toHaveBeenCalledTimes(1);

    q.enqueueInteractive("vectorize", { a: 9 });
    vi.advanceTimersByTime(300); // interactive runs

    // The interactive task ran; the queued background (a:2) was dropped.
    const ran = execute.mock.calls.map((c) => c[0]);
    expect(ran.some((t) => t.kind === "interactive" && t.params.a === 9)).toBe(true);
    expect(ran.some((t) => t.kind === "background" && t.params.a === 2)).toBe(false);

    d1.resolve();
    await vi.advanceTimersByTimeAsync(0);
  });

  it("dispose clears a stuck busy flag and pending work", async () => {
    const d = deferred();
    const execute = vi.fn(() => d.promise);
    const onBusyChange = vi.fn();
    const q = createTraceQueue({ execute, onBusyChange });

    q.enqueueInteractive("vectorize", { a: 1 });
    vi.advanceTimersByTime(300);
    expect(onBusyChange).toHaveBeenLastCalledWith("vectorize");

    q.dispose();
    expect(onBusyChange).toHaveBeenLastCalledWith(null);

    // Nothing enqueued after dispose.
    q.enqueueInteractive("vectorize", { a: 2 });
    vi.advanceTimersByTime(300);
    expect(execute).toHaveBeenCalledTimes(1);
  });
});

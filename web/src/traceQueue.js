import { stableStringify } from "./traceCache.js";

/**
 * Unified trace task queue for the editor.
 *
 * Two kinds of tasks, one scheduler:
 * - "interactive": user param change / preset select. Debounced (merges slider
 *   drags), then runs immediately, preempting everything: queued background
 *   tasks are dropped, and the engine cancels whatever is in flight for this
 *   image when the new request lands (request id + priority in traceSource).
 * - "background": pre-warm / backfill. Runs serially only while idle and is
 *   dropped the moment the user interacts.
 *
 * `execute(task)` performs the actual trace (cache-first) and applies the
 * result. It must treat TraceFlowError ("cancelled" / "busy") as a silent
 * no-op — that just means a newer task superseded it.
 */

const INTERACTIVE_DEBOUNCE_MS = 300;

export function createTraceQueue({ execute, onBusyChange }) {
  let pending = []; // queued background tasks, FIFO
  let inflightInteractive = null;
  let inflightBackground = null;
  let interactiveTimer = null;
  let seq = 0;
  let disposed = false;

  const sig = (params) => stableStringify(params);

  function startTask(task) {
    if (task.kind === "interactive") {
      inflightInteractive = task;
      onBusyChange?.(true);
    } else {
      inflightBackground = task;
    }
    execute(task).catch(() => {}).finally(() => {
      if (disposed) return;
      if (task.kind === "interactive") {
        // Only the newest interactive owns the busy flag.
        if (inflightInteractive === task) {
          inflightInteractive = null;
          onBusyChange?.(false);
        }
      } else if (inflightBackground === task) {
        inflightBackground = null;
      }
      pump();
    });
  }

  /** Run the next queued background task when nothing is in flight. */
  function pump() {
    if (disposed || inflightInteractive || inflightBackground) return;
    const next = pending.shift() || null;
    if (!next) return;
    startTask(next);
  }

  /**
   * User action: debounce (merges rapid slider changes), drop all queued
   * background work, then run immediately — the engine cancels whatever is
   * in flight for this image when the request lands.
   */
  function enqueueInteractive(params) {
    if (disposed) return;
    pending = [];
    if (interactiveTimer) clearTimeout(interactiveTimer);
    interactiveTimer = setTimeout(() => {
      interactiveTimer = null;
      if (disposed) return;
      startTask({ id: ++seq, kind: "interactive", params });
    }, INTERACTIVE_DEBOUNCE_MS);
  }

  /** Queue a background task (deduped by params); runs only while idle. */
  function enqueueBackground(params) {
    if (disposed) return;
    const s = sig(params);
    if (inflightBackground && sig(inflightBackground.params) === s) return;
    if (pending.some(t => sig(t.params) === s)) return;
    pending.push({ id: ++seq, kind: "background", params });
    pump();
  }

  function dispose() {
    disposed = true;
    if (interactiveTimer) clearTimeout(interactiveTimer);
    pending = [];
  }

  return { enqueueInteractive, enqueueBackground, dispose };
}

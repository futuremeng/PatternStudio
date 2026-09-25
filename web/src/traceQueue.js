import { stableStringify } from "./traceCache.js";

/**
 * Unified trace task queue for the editor.
 *
 * Two kinds of tasks, one scheduler:
 * - "interactive": user param change / preset select / manual processing.
 *   Debounced (merges slider drags), then runs immediately, preempting
 *   everything: queued background tasks are dropped, and the engine cancels
 *   whatever is in flight for this image when the new request lands (request
 *   id + priority in the source calls).
 * - "background": pre-warm / backfill. Runs serially only while idle and is
 *   dropped the moment the user interacts.
 *
 * Each task carries a `stage`:
 * - "vectorize": vectorization only (image → vectorized document preview).
 * - "layer": layering only (cached document → layered SVG, manual processing).
 * - "chain": vectorize + layer in sequence (background pre-warm).
 *
 * `execute(task)` performs the actual work (cache-first) and applies the
 * result. It must treat TraceFlowError ("cancelled" / "busy") as a silent
 * no-op — that just means a newer task superseded it.
 *
 * `onBusyChange(stage)` fires when an interactive task starts (with its
 * `stage`: "vectorize" | "layer") and when it finishes (with `null`), so the
 * UI can scope its busy indicators per stage. Background tasks never fire it.
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
      onBusyChange?.(task.stage);
    } else {
      inflightBackground = task;
    }
    execute(task).catch(() => {}).finally(() => {
      if (disposed) return;
      if (task.kind === "interactive") {
        // Only the newest interactive owns the busy flag.
        if (inflightInteractive === task) {
          inflightInteractive = null;
          onBusyChange?.(null);
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
   * `stage`: "vectorize" (param tweak / preset select) or "layer" (manual
   * processing).
   */
  function enqueueInteractive(stage, params) {
    if (disposed) return;
    pending = [];
    if (interactiveTimer) clearTimeout(interactiveTimer);
    interactiveTimer = setTimeout(() => {
      interactiveTimer = null;
      if (disposed) return;
      startTask({ id: ++seq, kind: "interactive", stage, params });
    }, INTERACTIVE_DEBOUNCE_MS);
  }

  /**
   * Queue a background pre-warm (vectorize + layer chain, deduped by params);
   * runs only while idle.
   */
  function enqueueBackground(params) {
    if (disposed) return;
    const s = sig(params);
    if (inflightBackground && inflightBackground.stage === "chain" && sig(inflightBackground.params) === s) return;
    if (pending.some(t => t.stage === "chain" && sig(t.params) === s)) return;
    pending.push({ id: ++seq, kind: "background", stage: "chain", params });
    pump();
  }

  function dispose() {
    disposed = true;
    if (interactiveTimer) clearTimeout(interactiveTimer);
    pending = [];
    // Release the busy flag if an interactive task is still in flight: its
    // finally will no longer run (disposed), so without this the flag would
    // stay stuck — e.g. a locale change recreates the queue without any new
    // trace being enqueued to clear it.
    if (inflightInteractive) onBusyChange?.(null);
  }

  return { enqueueInteractive, enqueueBackground, dispose };
}

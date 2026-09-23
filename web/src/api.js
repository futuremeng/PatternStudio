import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { translate } from "./i18n/index.jsx";
import { clearProjectCache } from "./traceCache.js";

const BASE = "/api/v1";

export const isTauri = () =>
  typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;

async function handle(res) {
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = await res.json();
      if (body.error) msg = body.error;
    } catch {
      const text = await res.text().catch(() => "");
      if (text) msg = text;
    }
    throw new Error(msg);
  }
  return res.json();
}

export async function health() {
  if (isTauri()) return { status: "ok", version: "desktop" };
  const res = await fetch(`${BASE}/health`);
  return handle(res);
}

/**
 * Resolve a traceable source once. Returns an opaque token reused across
 * traces so the image is read at most once:
 * - Tauri: the image file path (the engine reads it from disk and caches the
 *   decoded session, so no bytes cross the IPC bridge on every tweak).
 * - Web: the project id + filename (the server reads it from the project
 *   folder, so no re-upload on every parameter change).
 */
export function prepareSource(projectId, filename) {
  if (isTauri()) {
    return { kind: "file", path: `${projectId}/${filename}` };
  }
  return { kind: "project", projectId, filename };
}

/**
 * Trace flow error: the request was superseded, not failed.
 * `code` is "cancelled" (a newer request aborted this one on the engine side)
 * or "busy" (background work rejected because an interactive trace is running).
 */
export class TraceFlowError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "TraceFlowError";
    this.code = code;
  }
}

/** Client-generated id so the engine can match and supersede in-flight traces. */
export function newTraceRequestId() {
  return (typeof crypto !== "undefined" && crypto.randomUUID)
    ? crypto.randomUUID()
    : `req-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/**
 * Trace using a source prepared by {@link prepareSource}.
 * `opts`: { requestId, priority } — "interactive" (default) preempts any
 * in-flight trace of the same image; "background" (pre-warm) yields to it and
 * is rejected with a TraceFlowError("busy").
 */
export async function traceSource(source, params, opts = {}) {
  const { requestId, priority } = opts;
  if (isTauri()) {
    try {
      return await tauriInvoke("trace_file", {
        filePath: source.path, params,
        requestId: requestId ?? null, priority: priority ?? null,
      });
    } catch (e) {
      const msg = typeof e === "string" ? e : (e?.message || String(e));
      if (msg === "cancelled") throw new TraceFlowError("cancelled", msg);
      if (msg === "busy") throw new TraceFlowError("busy", msg);
      throw new Error(msg);
    }
  }
  const res = await fetch(`${BASE}/trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      asset_id: source.assetId ?? null,
      project_id: source.projectId ?? null, filename: source.filename ?? null, params,
      request_id: requestId ?? null, priority: priority ?? null,
    }),
  });
  if (res.status === 409) {
    let body = {};
    try { body = await res.json(); } catch { /* ignore */ }
    throw new TraceFlowError(body.busy ? "busy" : "cancelled", body.busy ? "trace busy" : "trace cancelled");
  }
  return handle(res);
}

/**
 * Smart Preset: analyze the image and trial-trace candidate parameter sets.
 * Returns { params, family, reasonKey, stats, candidates, elapsedMs } —
 * `params` is ready to feed into traceSource, and `reasonKey` ("scoped" |
 * "auto") is translated by the UI together with `stats`/`candidates`.
 */
export async function smartSource(source, family) {
  if (isTauri()) {
    return tauriInvoke("smart_file", { path: source.path, family: family || null });
  }
  const res = await fetch(`${BASE}/smart`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ project_id: source.projectId, filename: source.filename, family: family || null }),
  });
  return handle(res);
}

/**
 * Preprocess (digital-art enhancement) the source image with the Real-ESRGAN
 * anime model before tracing. Returns:
 * - `source`: a traceable source for the enhanced image (feed to traceSource)
 * - `previewUrl`: displayable URL of the enhanced image
 * - `width` / `height` / `elapsedMs` / `cached`
 * Results are cached on disk, so re-enabling for the same file is instant.
 */
export async function preprocessSource(source) {
  if (isTauri()) {
    const out = await tauriInvoke("preprocess_file", { filePath: source.path });
    const bytes = await tauriInvoke("read_image", { filePath: out.path });
    const blob = new Blob([new Uint8Array(bytes)], { type: "image/png" });
    return {
      source: { kind: "file", path: out.path },
      previewUrl: URL.createObjectURL(blob),
      width: out.width,
      height: out.height,
      elapsedMs: out.elapsedMs,
      cached: out.cached,
    };
  }
  const res = await fetch(`${BASE}/preprocess`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ project_id: source.projectId, filename: source.filename }),
  });
  const out = await handle(res);
  return {
    source: { kind: "asset", assetId: out.assetId },
    previewUrl: out.url,
    width: out.width,
    height: out.height,
    elapsedMs: out.elapsedMs,
    cached: out.cached,
  };
}

/** One-shot trace of an in-memory file (upload on web, bytes on Tauri). */
export async function traceImage(file, params) {
  if (isTauri()) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    return tauriInvoke("trace_bytes", { data: Array.from(bytes), params });
  }
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`${BASE}/uploads`, { method: "POST", body: form });
  const { asset_id } = await handle(res);
  const res2 = await fetch(`${BASE}/trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ asset_id, params }),
  });
  return handle(res2);
}

export async function openProject() {
  if (isTauri()) {
    const folderPath = await tauriInvoke("open_folder", { title: translate("projects.chooseFolder") });
    const images = await tauriInvoke("list_images", { folderPath });
    const name = folderPath.split("/").pop() || "project";
    try {
      await tauriInvoke("add_recent_project", { path: folderPath });
    } catch (e) {
      console.error("Failed to save recent project:", e);
    }
    return { id: folderPath, name, path: folderPath, imageCount: images.length, images: images.map(i => i.name) };
  }
  const path = prompt(translate("openFolderPrompt"));
  if (!path) throw new Error("cancelled");
  const res = await fetch(`${BASE}/projects`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
  return handle(res);
}

export async function listProjects() {
  if (isTauri()) {
    const recents = await tauriInvoke("recent_projects");
    const projects = [];
    for (const p of recents) {
      try {
        const images = await tauriInvoke("list_images", { folderPath: p.path });
        projects.push({
          id: p.path,
          name: p.name,
          path: p.path,
          imageCount: images.length,
          images: images.map(i => i.name),
        });
      } catch {
        // Folder no longer exists (moved/deleted); skip it.
      }
    }
    return { projects };
  }
  const res = await fetch(`${BASE}/projects`);
  return handle(res);
}

export async function deleteProject(projectId) {
  if (isTauri()) {
    await tauriInvoke("remove_recent_project", { path: projectId });
  } else {
    const res = await fetch(`${BASE}/projects/${projectId}`, { method: "DELETE" });
    if (!res.ok && res.status !== 204) throw new Error(`Failed to delete project`);
  }
  // Drop this project's cached trace results once it is gone. On desktop the
  // project id is the folder path, so without this, re-opening the same
  // folder (e.g. after replacing its images) would serve stale results.
  // Also drop the cached image list so thumbnails re-list the folder.
  await clearProjectCache(projectId);
  imageListCache.delete(projectId);
}

// Cache list_images results per folder so thumbnails don't re-list the
// directory on every load (a 100-image project would otherwise issue 100
// redundant IPC calls before a single byte of image data is read).
const imageListCache = new Map();
async function listImagesCached(folderPath) {
  if (imageListCache.has(folderPath)) return imageListCache.get(folderPath);
  const images = await tauriInvoke("list_images", { folderPath });
  imageListCache.set(folderPath, images);
  return images;
}

export async function getImageUrl(projectId, filename) {
  if (isTauri()) {
    const images = await listImagesCached(projectId);
    const img = images.find(i => i.name === filename);
    if (!img) throw new Error(`Image not found: ${filename}`);
    const bytes = await tauriInvoke("read_image", { filePath: img.path });
    const blob = new Blob([new Uint8Array(bytes)]);
    return URL.createObjectURL(blob);
  }
  return `${BASE}/projects/${projectId}/images/${encodeURIComponent(filename)}`;
}

export async function cancelBatchTrace(projectId, jobId) {
  if (isTauri()) {
    await tauriInvoke("cancel_batch_trace", { jobId });
    return;
  }
  await fetch(`${BASE}/projects/${projectId}/batch-trace/${jobId}/cancel`, { method: "POST" });
}

export function startBatchTrace(projectId, params, files, smart = false, family) {
  if (isTauri()) {
    let unlisten = null;
    let onEvent = () => {};
    let jobId = null;
    let resolveDone = () => {};
    const done = new Promise(r => { resolveDone = r; });
    const start = tauriInvoke("batch_trace", { folderPath: projectId, params, files, smart, family: family || null })
      .then((id) => {
        jobId = id;
        return tauriListen("batch-trace", (event) => {
          onEvent(event.payload);
          if (event.payload?.type === "complete" || event.payload?.type === "cancelled") resolveDone();
        }).then(u => { unlisten = u; });
      });
    return {
      start,
      done,
      setOnEvent: (fn) => { onEvent = fn; },
      close: () => { if (unlisten) { unlisten(); unlisten = null; } },
      cancel: () => {
        if (jobId) cancelBatchTrace(projectId, jobId).catch(() => {});
        if (unlisten) { unlisten(); unlisten = null; }
        resolveDone();
      },
    };
  }
  let eventSource = null;
  let onEvent = () => {};
  let jobId = null;
  let resolveDone = () => {};
  const done = new Promise(r => { resolveDone = r; });
  const start = fetch(`${BASE}/projects/${projectId}/batch-trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ params, files, smart, family: family || null }),
  }).then(async res => {
    const { job_id } = await res.json();
    jobId = job_id;
    eventSource = new EventSource(`${BASE}/projects/${projectId}/batch-trace/${job_id}/events`);
    // Server sends named SSE events (start/progress/complete); onmessage
    // only receives unnamed ones, so listen per event name.
    const onSse = (e) => {
      let data;
      try { data = JSON.parse(e.data); } catch { return; }
      // Normalize server {status: ...} payloads to Tauri-style {type: ...};
      // fall back to the SSE event name (start/complete have no status field).
      const evt = data.status ? { ...data, type: data.status } : { ...data, type: e.type };
      onEvent(evt);
      if (evt.type === "complete" || evt.type === "cancelled") resolveDone();
    };
    eventSource.addEventListener("start", onSse);
    eventSource.addEventListener("progress", onSse);
    eventSource.addEventListener("complete", onSse);
    eventSource.onerror = () => { eventSource.close(); resolveDone(); };
    return job_id;
  });
  return {
    start,
    done,
    setOnEvent: (fn) => { onEvent = fn; },
    close: () => { if (eventSource) { eventSource.close(); eventSource = null; } },
    cancel: () => {
      if (jobId) cancelBatchTrace(projectId, jobId).catch(() => {});
      if (eventSource) { eventSource.close(); eventSource = null; }
      resolveDone();
    },
  };
}

export async function generatePattern(params) {
  if (isTauri()) {
    const svg = await tauriInvoke("generate_pattern", { params });
    return { svg };
  }
  const res = await fetch(`${BASE}/pattern`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(params),
  });
  return handle(res);
}

export async function listPatterns() {
  if (isTauri()) {
    const patterns = await tauriInvoke("list_patterns");
    return { patterns };
  }
  const res = await fetch(`${BASE}/patterns`);
  return handle(res);
}

export async function downloadSvg(svg, name) {
  if (isTauri()) {
    await tauriInvoke("save_svg", { content: svg, suggestedName: name });
    return;
  }
  const blob = new Blob([svg], { type: "image/svg+xml" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

/**
 * Download arbitrary binary bytes (e.g. a ZIP of exported layer SVGs).
 * In the Tauri desktop app this opens a native save dialog via `save_bytes`;
 * in the browser it triggers a regular blob download.
 */
export async function downloadBlob(blob, name) {
  if (isTauri()) {
    const bytes = new Uint8Array(await blob.arrayBuffer());
    await tauriInvoke("save_bytes", { data: Array.from(bytes), suggestedName: name });
    return;
  }
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}

/** Base trace params; presets and user tweaks are merged on top of this. */
export const DEFAULT_PARAMS = {
  preset: "poster",
  clustering: "color",
  hierarchical: "stacked",
  colorPrecision: 6,
  layerDifference: 16,
  filterSpeckle: 4,
  mode: "spline",
  simplify: 0.1,
  pathPrecision: 2,
  maxColors: null,
  lineArt: false,
  binaryAdaptive: false,
  binaryThreshold: 128,
  binaryAdaptiveT: 15,
  binaryAdaptiveWindow: 0,
  watershedDetail: 128,
  palette: "",
  layers: 0,
  layerStrategy: "none",
  complexityThreshold: 70,
  cornerThreshold: 60,
  lengthThreshold: 4,
  optimize: 1,
};

/**
 * Bottom-strip presets: each one showcases a different algorithmic dimension
 * (output mode / color clustering / adaptive threshold / fine gradient / polygon fit).
 * Display labels live in the i18n dictionaries under `preset.<key>`.
 */
export const MODE_PRESETS = {
  lineArt: { params: { preset: "bw", clustering: "binary", lineArt: true, mode: "spline" } },
  color: { params: { preset: "poster", clustering: "color", mode: "spline" } },
  bw: { params: { preset: "bw", clustering: "binary", binaryAdaptive: true, mode: "spline", filterSpeckle: 4 } },
  gradient: { params: { preset: "photo", clustering: "color", colorPrecision: 8, layerDifference: 4, mode: "spline", simplify: 0.2 } },
  lowPoly: { params: { preset: "poster", clustering: "color", colorPrecision: 4, layerDifference: 32, mode: "polygon" } },
};

/** Full param set for a preset (DEFAULT_PARAMS + preset overrides). */
export function presetFullParams(key) {
  const preset = MODE_PRESETS[key];
  return preset ? { ...DEFAULT_PARAMS, ...preset.params } : { ...DEFAULT_PARAMS };
}

/**
 * Derive the preset family from a param set, so "smart tuning" can search
 * within the family the user is currently working in (whether they got here
 * via a preset card or manual tweaks).
 */
export function familyFromParams(p) {
  if (!p) return "color";
  if (p.lineArt && p.clustering === "binary") return "lineArt";
  if (p.clustering === "binary") return "bw";
  if (p.mode === "polygon") return "lowPoly";
  if (p.preset === "photo") return "gradient";
  return "color";
}


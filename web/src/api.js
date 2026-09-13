import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";

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

/** Trace using a source prepared by {@link prepareSource}. */
export async function traceSource(source, params) {
  if (isTauri()) {
    return tauriInvoke("trace_file", { filePath: source.path, params });
  }
  const res = await fetch(`${BASE}/trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ project_id: source.projectId, filename: source.filename, params }),
  });
  return handle(res);
}

/**
 * Smart Preset: analyze the image and trial-trace candidate parameter sets.
 * Returns { params, family, reason, stats, candidates, elapsedMs } —
 * `params` is ready to feed into traceSource, and `reason` explains the
 * choice in human-readable terms.
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
    const folderPath = await tauriInvoke("open_folder");
    const images = await tauriInvoke("list_images", { folderPath });
    const name = folderPath.split("/").pop() || "project";
    try {
      await tauriInvoke("add_recent_project", { path: folderPath });
    } catch (e) {
      console.error("Failed to save recent project:", e);
    }
    return { id: folderPath, name, path: folderPath, imageCount: images.length, images: images.map(i => i.name) };
  }
  const path = prompt("输入项目文件夹路径:");
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
    return;
  }
  const res = await fetch(`${BASE}/projects/${projectId}`, { method: "DELETE" });
  if (!res.ok && res.status !== 204) throw new Error(`Failed to delete project`);
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
 */
export const MODE_PRESETS = {
  lineArt: { label: "线稿", params: { preset: "bw", clustering: "binary", lineArt: true, mode: "spline" } },
  color: { label: "彩色填充", params: { preset: "poster", clustering: "color", mode: "spline" } },
  bw: { label: "黑白", params: { preset: "bw", clustering: "binary", binaryAdaptive: true, mode: "spline", filterSpeckle: 4 } },
  gradient: { label: "渐变", params: { preset: "photo", clustering: "color", colorPrecision: 8, layerDifference: 4, mode: "spline", simplify: 0.2 } },
  lowPoly: { label: "低多边形", params: { preset: "poster", clustering: "color", colorPrecision: 4, layerDifference: 32, mode: "polygon" } },
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


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
  if (isTauri()) return { projects: [] };
  const res = await fetch(`${BASE}/projects`);
  return handle(res);
}

export async function deleteProject(projectId) {
  if (isTauri()) return;
  const res = await fetch(`${BASE}/projects/${projectId}`, { method: "DELETE" });
  if (!res.ok && res.status !== 204) throw new Error(`Failed to delete project`);
}

export async function getImageUrl(projectId, filename) {
  if (isTauri()) {
    const images = await tauriInvoke("list_images", { folderPath: projectId });
    const img = images.find(i => i.name === filename);
    if (!img) throw new Error(`Image not found: ${filename}`);
    const bytes = await tauriInvoke("read_image", { filePath: img.path });
    const blob = new Blob([new Uint8Array(bytes)]);
    return URL.createObjectURL(blob);
  }
  return `${BASE}/projects/${projectId}/images/${encodeURIComponent(filename)}`;
}

export function startBatchTrace(projectId, params) {
  if (isTauri()) {
    let unlisten = null;
    let onEvent = () => {};
    const start = tauriInvoke("batch_trace", { folderPath: projectId, params })
      .then(() => tauriListen("batch-trace", (event) => { onEvent(event.payload); })
        .then(u => { unlisten = u; }));
    return { start, setOnEvent: (fn) => { onEvent = fn; }, cancel: () => { if (unlisten) unlisten(); } };
  }
  let eventSource = null;
  let onEvent = () => {};
  const start = fetch(`${BASE}/projects/${projectId}/batch-trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ params }),
  }).then(async res => {
    const { job_id } = await res.json();
    eventSource = new EventSource(`${BASE}/projects/${projectId}/batch-trace/${job_id}/events`);
    eventSource.onmessage = (e) => { try { onEvent(JSON.parse(e.data)); } catch {} };
    eventSource.onerror = () => { eventSource.close(); };
    return job_id;
  });
  return { start, setOnEvent: (fn) => { onEvent = fn; }, cancel: () => { if (eventSource) eventSource.close(); } };
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

export const MODE_PRESETS = {
  original: { label: "原图", params: {} },
  bw: { label: "黑白", params: { preset: "bw", clustering: "binary" } },
  lineArt: { label: "线稿", params: { preset: "bw", clustering: "binary", lineArt: true } },
  color: { label: "彩色填充", params: { preset: "poster", clustering: "color" } },
  silhouette: { label: "剪影", params: { preset: "bw", clustering: "binary", maxColors: 1 } },
  gradient: { label: "渐变", params: { preset: "photo", clustering: "color", colorPrecision: 8, layerDifference: 4 } },
};


import { invoke as tauriInvoke } from "@tauri-apps/api/core";

const BASE = "/api/v1";

/** True when running inside the Tauri desktop webview. */
export const isTauri = () =>
  typeof window !== "undefined" && !!window.__TAURI_INTERNALS__;

async function handle(res) {
  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const body = await res.json();
      if (body.error) msg = body.error;
    } catch {
      // body is a plain string
      const text = await res.text().catch(() => "");
      if (text) msg = text;
    }
    throw new Error(msg);
  }
  return res.json();
}

export async function uploadFile(file) {
  const form = new FormData();
  form.append("file", file);
  const res = await fetch(`${BASE}/uploads`, { method: "POST", body: form });
  return handle(res);
}

/**
 * Trace an image file. In the desktop app the bytes go straight to the
 * in-process Rust engine; in the browser the file is uploaded to the API
 * server first. Returns the same shape in both modes.
 */
export async function traceImage(file, params) {
  if (isTauri()) {
    const bytes = new Uint8Array(await file.arrayBuffer());
    return tauriInvoke("trace_bytes", { data: Array.from(bytes), params });
  }
  const { asset_id } = await uploadFile(file);
  const res = await fetch(`${BASE}/trace`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ asset_id: asset_id, ...params }),
  });
  return handle(res);
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
  const data = await handle(res);
  return data;
}

export async function listPatterns() {
  if (isTauri()) {
    const patterns = await tauriInvoke("list_patterns");
    return { patterns };
  }
  const res = await fetch(`${BASE}/patterns`);
  return handle(res);
}

export async function health() {
  if (isTauri()) {
    return { status: "ok", version: "desktop" };
  }
  const res = await fetch(`${BASE}/health`);
  return handle(res);
}

/**
 * Save an SVG to disk. Uses a native save dialog in the desktop app,
 * a browser download otherwise.
 */
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


// Pure helpers for the image editor: SVG snapshotting, color stats, and the
// persisted "last preset" choice. No React, no component state.

import { MODE_PRESETS, presetFullParams } from "./api.js";

// Serialize the current (edited) preview SVG into a standalone SVG whose single
// <g> holds every path exactly as it is now — moves applied, deleted patches
// removed, and selection highlights / control points / marquee stripped out.
// Used to snapshot the user's edits as a new, exportable layer.
export function serializeEditedSvg(svg) {
  const vb = svg.viewBox.baseVal;
  const X = vb.x || 0;
  const Y = vb.y || 0;
  const W = vb.width;
  const H = vb.height;
  const serializer = new XMLSerializer();
  const parts = [];
  svg.querySelectorAll("path").forEach((p) => {
    const clone = p.cloneNode(true);
    clone.removeAttribute("style"); // drop inline selection highlight
    // XMLSerializer re-declares the SVG namespace on each element; the root
    // <svg> already declares it, so strip the redundant attribute for a clean file.
    const str = serializer
      .serializeToString(clone)
      .replace(/ xmlns="http:\/\/www\.w3\.org\/2000\/svg"/g, "");
    parts.push("    " + str);
  });
  return (
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${X} ${Y} ${W} ${H}" width="${W}" height="${H}">\n` +
    `  <g id="edited-layer" data-role="edited">\n` +
    parts.join("\n") +
    `\n  </g>\n</svg>\n`
  );
}

// Most common non-"none" fill color among a set of path elements.
export function dominantFill(paths) {
  const counts = {};
  for (const p of paths) {
    const f = p.getAttribute("fill");
    if (f && f !== "none") counts[f] = (counts[f] || 0) + 1;
  }
  let best = null;
  let bestN = 0;
  for (const [c, n] of Object.entries(counts)) {
    if (n > bestN) {
      best = c;
      bestN = n;
    }
  }
  return best || "#6f7683";
}

// The last selected system preset, persisted so that any image opened from
// the list (same or different project) starts with it. The desktop app
// embeds this same frontend, so it shares the choice.
const LAST_PRESET_KEY = "patternstudio.lastPreset";
export function loadLastPreset() {
  try {
    const key = localStorage.getItem(LAST_PRESET_KEY);
    return key && MODE_PRESETS[key] ? key : null;
  } catch { return null; }
}
export function saveLastPreset(key) {
  try { localStorage.setItem(LAST_PRESET_KEY, key); } catch { /* storage unavailable */ }
}

/**
 * Normalize an error thrown by the API layer into a display string. Tauri
 * command rejections are plain strings (not Error objects), so `e.message`
 * would be undefined there; the web path throws real Error objects.
 */
export function errMsg(e) {
  return typeof e === "string" ? e : (e?.message || String(e));
}

/** Default configs from MODE_PRESETS (full param sets, ready to trace). */
export function buildDefaultConfigs(presetLabel) {
  return Object.entries(MODE_PRESETS).map(([key]) => ({
    id: `default-${key}`,
    label: presetLabel(key),
    params: presetFullParams(key),
    isDefault: true,
  }));
}

// Pure helpers for the image editor: SVG snapshotting, color stats, and the
// persisted "last preset" choice. No React, no component state.

import { MODE_PRESETS, presetFullParams } from "./api.js";

/**
 * The canvas box a layer snapshot must be emitted with.
 *
 * Separating a layer takes a *subset* of the shapes, it does not crop the
 * artwork: every shape keeps its original image-space coordinates, so the
 * canvas has to stay the size of the source image. `canvasSize` (the trace
 * result's width/height) is the source of truth. When it is unknown we fall
 * back to the box the snapshot already carries (`source` is either an SVG
 * markup string or a live <svg> element) so nothing regresses.
 */
export function canvasBox(source, canvasSize) {
  if (canvasSize && canvasSize.w > 0 && canvasSize.h > 0) {
    return { x: 0, y: 0, w: canvasSize.w, h: canvasSize.h };
  }
  if (typeof source === "string") {
    const m = source.match(/viewBox="(-?[\d.]+) (-?[\d.]+) ([\d.]+) ([\d.]+)"/);
    if (m) return { x: Number(m[1]), y: Number(m[2]), w: Number(m[3]), h: Number(m[4]) };
  } else if (source) {
    const vb = source.viewBox?.baseVal;
    if (vb && vb.width > 0) return { x: vb.x || 0, y: vb.y || 0, w: vb.width, h: vb.height };
  }
  return { x: 0, y: 0, w: 100, h: 100 };
}

/**
 * Rewrite the root <svg> canvas box (viewBox + width/height) of a markup
 * string, leaving every shape untouched. Moving the canvas box of a layer
 * snapshot never moves the artwork — the shapes live in image space — it only
 * restores the canvas they were cut out of.
 */
export function withCanvasBox(svgMarkup, box) {
  const attrs =
    `viewBox="${box.x} ${box.y} ${box.w} ${box.h}" width="${box.w}" height="${box.h}"`;
  return svgMarkup.replace(/<svg\b[^>]*>/, (tag) => {
    const selfClosing = /\/\s*>$/.test(tag);
    const bare = tag
      .replace(/\s(?:viewBox|width|height)="[^"]*"/gi, "")
      .replace(/\s*\/?\s*>$/, "");
    return `${bare} ${attrs}${selfClosing ? "/>" : ">"}`;
  });
}

// Serialize the current (edited) preview SVG into a standalone SVG whose single
// <g> holds every path exactly as it is now — moves applied, deleted patches
// removed, and selection highlights / control points / marquee stripped out.
// Used to snapshot the user's edits as a new, exportable layer. `canvasSize`
// (when known) pins the snapshot to the original canvas so editing crops never
// shrink the layer's canvas.
export function serializeEditedSvg(svg, canvasSize) {
  const { x: X, y: Y, w: W, h: H } = canvasBox(svg, canvasSize);
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

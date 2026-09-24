import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, prepareSource, traceSource, smartSource, newTraceRequestId, downloadSvg, MODE_PRESETS, DEFAULT_PARAMS, presetFullParams, familyFromParams } from "../api.js";
import { getCachedTrace, setCachedTrace, stableStringify } from "../traceCache.js";
import { createTraceQueue } from "../traceQueue.js";
import { loadSavedConfigs, saveConfig, deleteSavedConfig } from "../configStore.js";
import ParameterPanel from "../components/ParameterPanel.jsx";
import LayerPanel from "../components/LayerPanel.jsx";
import ConfigStrip from "../components/ConfigStrip.jsx";
import AppHeader from "../components/AppHeader.jsx";
import AdvancedSvgEditor from "../components/AdvancedSvgEditor.jsx";
import CompareViewer, { useViewportControls } from "../components/CompareViewer.jsx";
import PreprocessModal from "../components/PreprocessModal.jsx";
import { IconMaximize, IconRestore, IconSelect, IconZoomIn, IconZoomOut, IconFit, IconAdvanced } from "../components/icons.jsx";
import { useI18n } from "../i18n/index.jsx";

// Wait this long after a trace settles before snapshotting a user config (ms)
const CONFIG_SETTLE_MS = 1500;
const MAX_USER_CONFIGS = 12;
// Pre-warm: presets to pre-generate in the background after the initial trace,
// in priority order. The currently-active preset is already cached and skipped.
const PREWARM_ORDER = ["color", "lineArt", "bw", "gradient", "lowPoly"];

// Serialize the current (edited) preview SVG into a standalone SVG whose single
// <g> holds every path exactly as it is now — moves applied, deleted patches
// removed, and selection highlights / control points / marquee stripped out.
// Used to snapshot the user's edits as a new, exportable layer.
function serializeEditedSvg(svg) {
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
function dominantFill(paths) {
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
function loadLastPreset() {
  try {
    const key = localStorage.getItem(LAST_PRESET_KEY);
    return key && MODE_PRESETS[key] ? key : null;
  } catch { return null; }
}
function saveLastPreset(key) {
  try { localStorage.setItem(LAST_PRESET_KEY, key); } catch { /* storage unavailable */ }
}

/**
 * Normalize an error thrown by the API layer into a display string. Tauri
 * command rejections are plain strings (not Error objects), so `e.message`
 * would be undefined there; the web path throws real Error objects.
 */
function errMsg(e) {
  return typeof e === "string" ? e : (e?.message || String(e));
}

/** Default configs from MODE_PRESETS (full param sets, ready to trace). */
function buildDefaultConfigs(presetLabel) {
  return Object.entries(MODE_PRESETS).map(([key]) => ({
    id: `default-${key}`,
    label: presetLabel(key),
    params: presetFullParams(key),
    isDefault: true,
  }));
}

export default function ImageEditorPage() {
  const { t, presetLabel, locale } = useI18n();
  const { projectId, filename } = useParams();
  const navigate = useNavigate();
  const [imageUrl, setImageUrl] = useState(null);
  const [params, setParams] = useState(() => {
    const key = loadLastPreset();
    return key ? presetFullParams(key) : DEFAULT_PARAMS;
  });
  const [result, setResult] = useState(null);
  // Layers the user has snapshotted from the edited preview (see saveAsLayer).
  // Kept separate from result.layers so a re-trace can reset them cleanly.
  const [savedLayers, setSavedLayers] = useState([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  // What the error banner is about, so its retry button re-runs the right thing.
  const [errorKind, setErrorKind] = useState(null); // "trace" | "smart" | "load"
  // Bumped to re-run the image-load effect (retry after a load failure).
  const [reloadTick, setReloadTick] = useState(0);
  const [configs, setConfigs] = useState(() => buildDefaultConfigs(presetLabel));
  const [activeConfigId, setActiveConfigId] = useState(() => {
    const key = loadLastPreset();
    return key ? `default-${key}` : "default-color";
  });
  const [compareMode, setCompareMode] = useState("side"); // "side" | "overlay"
  const [smartInfo, setSmartInfo] = useState(null); // last Smart Preset decision
  const [smartBusy, setSmartBusy] = useState(false);
  // Digital-art enhancement modal (Real-ESRGAN anime): iterative enhancement
  // with per-iteration thumbnails; saving writes a new image into the project
  // folder and switches the editor to it.
  const [preprocessOpen, setPreprocessOpen] = useState(false);
  const configTimerRef = useRef(null);
  const userConfigSeq = useRef(0);
  const configsRef = useRef(configs);
  const imageId = `${projectId}/${filename}`;
  const imageIdRef = useRef(imageId);
  useEffect(() => { imageIdRef.current = imageId; }, [imageId]);

  // --- Preview view controls: maximize ---
  const [maximized, setMaximized] = useState(false);
  const [tool, setTool] = useState("none");
  const [advancedMode, setAdvancedMode] = useState(false);
  // Viewport of the advanced-mode branch (CompareViewer owns its own viewport).
  const advViewportRef = useRef(null);
  // Imperative handle of the advanced editor (zoomIn/zoomOut/zoomFit).
  const advEditorRef = useRef(null);
  // Set before a non-retrace result update (advanced editor "done") so the
  // saved-layers reset effect below skips that one change.
  const skipLayerResetRef = useRef(false);
  // Natural size of the source image (known as soon as it loads, before the
  // first trace finishes). Used to size the vector placeholder and enable
  // zoom/fit controls from the very first paint.
  const [srcSize, setSrcSize] = useState({ w: 0, h: 0 });

  // --- Patch selection & dragging ---
  const svgContainerRef = useRef(null);
  const pathTransformsRef = useRef(new Map()); // pathIdx → { dx, dy } in SVG coords
  const dragRef = useRef(null); // { idx, startClientX, startClientY, origDx, origDy }
  const selectedPathIdxsRef = useRef(new Set()); // Set of selected path indices

  // In advanced mode the preview stage is replaced by the advanced editor's
  // own canvas; the page keeps the viewport chrome for that branch (wheel
  // zoom routed to the editor, drag-to-pan).
  useViewportControls({
    viewportRef: advViewportRef,
    enabled: advancedMode && !!result?.svg,
    onWheelZoom: (dir) => {
      if (dir) advEditorRef.current?.zoomIn();
      else advEditorRef.current?.zoomOut();
    },
  });

  // Reset the view when switching to a different image. The CompareViewer is
  // keyed by imageId so its zoom / overlay-divider state resets with it.
  useEffect(() => { setAdvancedMode(false); }, [imageId]);

  const toggleMaximize = useCallback(() => setMaximized(m => !m), []);

  useEffect(() => { configsRef.current = configs; }, [configs]);

  // Keep the default preset labels in sync with the active locale.
  useEffect(() => {
    setConfigs(prev => prev.map(c =>
      c.isDefault ? { ...c, label: presetLabel(c.id.slice("default-".length)) } : c
    ));
  }, [locale, presetLabel]);

  // Restore user configs persisted from previous sessions.
  // Thumbnails are refilled from the trace cache when available.
  useEffect(() => {
    let cancelled = false;
    loadSavedConfigs().then(async saved => {
      if (cancelled || !saved.length) return;
      const restored = saved.map(s => ({
        id: s.id, label: s.label, params: { ...DEFAULT_PARAMS, ...s.params },
        isDefault: false, saved: true, result: null,
      }));
      for (const c of restored) {
        c.result = await getCachedTrace(imageId, c.params).catch(() => null);
      }
      if (cancelled) return;
      setConfigs(prev => {
        const known = new Set(prev.map(c => c.id));
        const fresh = restored.filter(c => !known.has(c.id));
        return fresh.length ? [...prev, ...fresh] : prev;
      });
    }).catch(() => {});
    return () => { cancelled = true; };
  }, [imageId]);

  // Load the source image; the auto-trace effect below picks up from here.
  useEffect(() => {
    let cancelled = false;
    setSrcSize({ w: 0, h: 0 });
    (async () => {
      try {
        const url = await getImageUrl(projectId, filename);
        if (cancelled) return;
        setImageUrl(url);
      } catch (e) {
        if (!cancelled) { setError(errMsg(e)); setErrorKind("load"); }
      }
    })();
    return () => { cancelled = true; };
  }, [projectId, filename, reloadTick]);

  // --- Patch selection & dragging helpers ---

  const getSvgAndPaths = useCallback(() => {
    const container = svgContainerRef.current;
    if (!container) return null;
    const svg = container.querySelector("svg");
    if (!svg) return null;
    return { svg, paths: Array.from(svg.querySelectorAll("path")) };
  }, []);

  // Snapshot the current edited preview as a new, exportable layer.
  const saveAsLayer = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    const svgStr = serializeEditedSvg(res.svg);
    setSavedLayers((prev) => [
      ...prev,
      {
        name: t("layer.editedLayer", { n: prev.length + 1 }),
        role: "edited",
        shapeCount: res.paths.length,
        dominantColor: dominantFill(res.paths),
        previewSvg: svgStr,
        profile: null,
      },
    ]);
  }, [getSvgAndPaths, t]);

  // A new trace (or cached/preset result) replaces the working canvas, so any
  // previously-saved edited layers no longer apply — clear them. The one
  // exception is the advanced editor's "done" (a non-retrace svg update),
  // which flags skipLayerResetRef so its snapshots survive.
  useEffect(() => {
    if (skipLayerResetRef.current) {
      skipLayerResetRef.current = false;
      return;
    }
    setSavedLayers([]);
  }, [result]);

  // Snapshot the advanced editor's canvas as a new, exportable layer.
  const saveAdvancedAsLayer = useCallback((svgStr) => {
    try {
      const doc = new DOMParser().parseFromString(svgStr, "image/svg+xml");
      const paths = Array.from(doc.querySelectorAll("path"));
      setSavedLayers((prev) => [
        ...prev,
        {
          name: t("layer.advLayer", { n: prev.length + 1 }),
          role: "edited",
          shapeCount: paths.length,
          dominantColor: dominantFill(paths),
          previewSvg: svgStr,
          profile: null,
        },
      ]);
    } catch { /* ignore an unparseable snapshot */ }
  }, [t]);

  const clearPatchSelection = useCallback(() => {
    selectedPathIdxsRef.current = new Set();
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    paths.forEach(p => {
      p.style.stroke = "";
      p.style.strokeWidth = "";
      p.style.cursor = "";
    });
    svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
  }, [getSvgAndPaths]);

  // Highlight every path in the current selection set.
  const highlightPaths = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    const sel = selectedPathIdxsRef.current;
    res.paths.forEach((p, i) => {
      if (sel.has(i)) {
        p.style.stroke = "#41b6e6";
        p.style.strokeWidth = "2";
        p.style.cursor = "move";
      } else {
        p.style.stroke = "";
        p.style.strokeWidth = "";
        p.style.cursor = "pointer";
      }
    });
  }, [getSvgAndPaths]);

  // Render the 8 control points for each selected path.
  const renderControlPoints = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
    for (const idx of selectedPathIdxsRef.current) {
      const path = paths[idx];
      if (!path) continue;
      const t = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
      const bb = path.getBBox();
      const r = Math.max(3, Math.min(bb.width, bb.height) * 0.015);
      const mx = bb.x + bb.width / 2 + t.dx;
      const my = bb.y + bb.height / 2 + t.dy;
      const pts = [
        [bb.x + t.dx, bb.y + t.dy], [mx, bb.y + t.dy],
        [bb.x + bb.width + t.dx, bb.y + t.dy], [bb.x + bb.width + t.dx, my],
        [bb.x + bb.width + t.dx, bb.y + bb.height + t.dy], [mx, bb.y + bb.height + t.dy],
        [bb.x + t.dx, bb.y + bb.height + t.dy], [bb.x + t.dx, my],
      ];
      for (const [px, py] of pts) {
        const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
        c.setAttribute("cx", px); c.setAttribute("cy", py); c.setAttribute("r", r);
        c.setAttribute("fill", "#41b6e6"); c.setAttribute("stroke", "#fff");
        c.setAttribute("stroke-width", (r * 0.35).toFixed(1));
        c.classList.add("patch-cp"); c.style.pointerEvents = "none";
        svg.appendChild(c);
      }
    }
  }, [getSvgAndPaths]);

  // Reset patch selection when the SVG changes (new trace result).
  useEffect(() => {
    selectedPathIdxsRef.current = new Set();
    pathTransformsRef.current.clear();
  }, [result?.svg]);

  // Attach pointer interactions to SVG paths for patch selection & dragging.
  useEffect(() => {
    if (tool !== "select" || !result?.svg) return;
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    if (!paths.length) return;

    // Re-apply existing transforms & highlight (e.g. after compare-mode switch)
    pathTransformsRef.current.forEach((t, idx) => {
      if (paths[idx]) paths[idx].setAttribute("transform", `translate(${t.dx}, ${t.dy})`);
    });
    if (selectedPathIdxsRef.current.size > 0) {
      highlightPaths();
      renderControlPoints();
    }

    // Convert a client point to SVG user-space coordinates. Uses the
    // element's actual screen CTM, which stays correct for any viewBox
    // (including the offset viewBox produced by "shrink canvas") and any
    // letterboxing — a manual rect/viewBox ratio would drift as soon as the
    // viewBox aspect ratio differs from the rendered element's.
    const clientToSvg = (clientX, clientY) => {
      const ctm = svg.getScreenCTM();
      if (!ctm) return null;
      const p = new DOMPoint(clientX, clientY).matrixTransform(ctm.inverse());
      return { x: p.x, y: p.y };
    };

    // Select every path whose (transformed) bounding box intersects the
    // marquee rect. With `additive` the result is unioned with the current
    // selection (⌘/Ctrl-drag); otherwise it replaces it.
    const applyMarqueeSelection = (rect, additive) => {
      const sel = additive ? new Set(selectedPathIdxsRef.current) : new Set();
      paths.forEach((p, i) => {
        const t = pathTransformsRef.current.get(i) || { dx: 0, dy: 0 };
        let bb;
        try { bb = p.getBBox(); } catch { return; } // skip detached/hidden paths
        const x0 = bb.x + t.dx, y0 = bb.y + t.dy;
        const x1 = bb.x + bb.width + t.dx, y1 = bb.y + bb.height + t.dy;
        if (x1 < rect.x || x0 > rect.x + rect.w || y1 < rect.y || y0 > rect.y + rect.h) return;
        sel.add(i);
      });
      selectedPathIdxsRef.current = sel;
      highlightPaths();
      renderControlPoints();
    };

    const onPathPointerDown = (e) => {
      if (e.button !== 0) return;
      const idx = paths.indexOf(e.target);
      if (idx === -1) return;
      e.preventDefault();
      e.stopPropagation();
      // ⌘/Ctrl+click: toggle this path in/out of the selection, no dragging.
      if (e.metaKey || e.ctrlKey) {
        const sel = new Set(selectedPathIdxsRef.current);
        if (sel.has(idx)) sel.delete(idx); else sel.add(idx);
        selectedPathIdxsRef.current = sel;
        highlightPaths();
        renderControlPoints();
        return;
      }
      selectedPathIdxsRef.current = new Set([idx]);
      highlightPaths();
      renderControlPoints();
      const t = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
      dragRef.current = { idx, sx: e.clientX, sy: e.clientY, ox: t.dx, oy: t.dy };

      const onMove = (me) => {
        const d = dragRef.current;
        if (!d) return;
        const ctm = svg.getScreenCTM();
        if (!ctm) return;
        const inv = ctm.inverse();
        const start = new DOMPoint(d.sx, d.sy).matrixTransform(inv);
        const cur = new DOMPoint(me.clientX, me.clientY).matrixTransform(inv);
        const dx = d.ox + (cur.x - start.x);
        const dy = d.oy + (cur.y - start.y);
        pathTransformsRef.current.set(d.idx, { dx, dy });
        paths[d.idx].setAttribute("transform", `translate(${dx}, ${dy})`);
        renderControlPoints();
      };
      const onUp = () => {
        dragRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    // Marquee (rubber-band) selection: press on empty svg area and drag.
    // Paths stop propagation on pointerdown, so this only fires for the
    // background. A plain click on the background clears the selection.
    const onSvgPointerDown = (e) => {
      if (e.button !== 0) return;
      e.stopPropagation(); // don't let the viewport start panning
      const start = clientToSvg(e.clientX, e.clientY);
      if (!start) return;
      const additive = e.metaKey || e.ctrlKey;
      let marquee = null;
      let active = false;
      let last = start;

      const onMove = (me) => {
        const cur = clientToSvg(me.clientX, me.clientY);
        if (!cur) return;
        last = cur;
        if (!active) {
          if (Math.hypot(me.clientX - e.clientX, me.clientY - e.clientY) < 3) return;
          active = true;
          marquee = document.createElementNS("http://www.w3.org/2000/svg", "rect");
          marquee.setAttribute("class", "patch-marquee");
          marquee.setAttribute("fill", "rgba(65, 182, 230, 0.15)");
          marquee.setAttribute("stroke", "#41b6e6");
          marquee.setAttribute("stroke-width", "1");
          marquee.setAttribute("stroke-dasharray", "4 3");
          marquee.style.pointerEvents = "none";
          svg.appendChild(marquee);
        }
        const x = Math.min(start.x, cur.x);
        const y = Math.min(start.y, cur.y);
        const w = Math.abs(cur.x - start.x);
        const h = Math.abs(cur.y - start.y);
        marquee.setAttribute("x", x);
        marquee.setAttribute("y", y);
        marquee.setAttribute("width", w);
        marquee.setAttribute("height", h);
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        if (marquee) marquee.remove();
        if (active) {
          const x = Math.min(start.x, last.x);
          const y = Math.min(start.y, last.y);
          applyMarqueeSelection(
            { x, y, w: Math.abs(last.x - start.x), h: Math.abs(last.y - start.y) },
            additive
          );
        } else if (!additive) {
          clearPatchSelection();
        }
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    const onKeyDown = (e) => {
      if (e.key === "Escape") { clearPatchSelection(); return; }
      if ((e.key === "Delete" || e.key === "Backspace") && selectedPathIdxsRef.current.size > 0) {
        e.preventDefault();
        const current = getSvgAndPaths();
        if (!current) return;
        // Remove in descending order so earlier indices stay valid.
        const removed = [...selectedPathIdxsRef.current].sort((a, b) => b - a);
        for (const idx of removed) {
          if (current.paths[idx]) current.paths[idx].remove();
          pathTransformsRef.current.delete(idx);
        }
        // Removing paths shifts the remaining indices: remap transforms and
        // the selection set accordingly.
        const remap = (i) => i - removed.filter(r => r < i).length;
        const newTransforms = new Map();
        pathTransformsRef.current.forEach((t, i) => newTransforms.set(remap(i), t));
        pathTransformsRef.current = newTransforms;
        selectedPathIdxsRef.current = new Set([...selectedPathIdxsRef.current].map(remap));
        highlightPaths();
        renderControlPoints();
      }
    };

    paths.forEach(p => {
      p.style.pointerEvents = "auto";
      p.style.cursor = "pointer";
      p.style.touchAction = "none";
      p.addEventListener("pointerdown", onPathPointerDown);
    });
    svg.addEventListener("pointerdown", onSvgPointerDown);
    window.addEventListener("keydown", onKeyDown);

    return () => {
      paths.forEach(p => {
        p.style.pointerEvents = "";
        p.style.cursor = "";
        p.style.touchAction = "";
        p.removeEventListener("pointerdown", onPathPointerDown);
      });
      svg.removeEventListener("pointerdown", onSvgPointerDown);
      window.removeEventListener("keydown", onKeyDown);
      dragRef.current = null;
    };
  }, [tool, result?.svg, compareMode, getSvgAndPaths, highlightPaths, renderControlPoints, clearPatchSelection]);

  /**
   * Write a trace result back to any config whose params match, so the
   * strip thumbnails (presets / saved configs) update without a reload.
   * No-op when no config matches (plain user tweaks).
   */
  const applyResultToConfigs = useCallback((traceParams, res) => {
    const sig = stableStringify(traceParams);
    setConfigs(prev => prev.some(c => stableStringify(c.params) === sig)
      ? prev.map(c => (stableStringify(c.params) === sig ? { ...c, result: res } : c))
      : prev);
  }, []);

  /**
   * Snapshot the current params as a user config once the user stops
   * adjusting (settled for CONFIG_SETTLE_MS). Skipped when the params
   * already match an existing config.
   */
  const scheduleUserConfig = useCallback((traceParams, res) => {
    if (configTimerRef.current) clearTimeout(configTimerRef.current);
    configTimerRef.current = setTimeout(() => {
      configTimerRef.current = null;
      const sig = stableStringify(traceParams);
      // Skip when the params already match an existing config
      // (e.g. initial load matches the "彩色填充" default).
      if (configsRef.current.some(c => stableStringify(c.params) === sig)) return;
      userConfigSeq.current += 1;
      const cfg = {
        id: `user-${Date.now()}-${userConfigSeq.current}`,
        label: t("editor.customConfig", { n: userConfigSeq.current }),
        params: traceParams,
        isDefault: false,
        saved: false,
        result: res,
      };
      setConfigs(prev => {
        if (prev.some(c => stableStringify(c.params) === sig)) return prev;
        // Evict the oldest *unsaved* snapshot; saved configs are kept.
        const unsaved = prev.filter(c => !c.isDefault && !c.saved);
        const next = unsaved.length >= MAX_USER_CONFIGS
          ? prev.filter(c => c.id !== unsaved[0].id)
          : prev;
        return [...next, cfg];
      });
      setActiveConfigId(cfg.id);
    }, CONFIG_SETTLE_MS);
  }, [t]);

  /**
   * Run one queue task: cache-first trace, then apply the result.
   * Interactive tasks update the main preview; background tasks only refresh
   * matching config thumbnails. Superseded tasks (cancelled / busy) are a
   * silent no-op — a newer task already owns the result.
   */
  const executeTask = useCallback(async (task) => {
    const imageIdNow = imageId;
    const source = prepareSource(projectId, filename);
    const cached = await getCachedTrace(imageIdNow, task.params).catch(() => null);
    if (imageIdRef.current !== imageIdNow) return; // image switched mid-flight
    if (cached) {
      applyResultToConfigs(task.params, cached);
      if (task.kind === "interactive") {
        setResult(cached);
        setError(null);
        setErrorKind(null);
        scheduleUserConfig(task.params, cached);
      }
      return;
    }
    try {
      const res = await traceSource(source, task.params, {
        requestId: newTraceRequestId(),
        priority: task.kind === "interactive" ? "interactive" : "background",
      });
      if (imageIdRef.current !== imageIdNow) return;
      setCachedTrace(imageIdNow, task.params, res).catch(() => {});
      applyResultToConfigs(task.params, res);
      if (task.kind === "interactive") {
        setResult(res);
        setError(null);
        setErrorKind(null);
        scheduleUserConfig(task.params, res);
      }
    } catch (e) {
      if (imageIdRef.current !== imageIdNow) return;
      if (e?.code === "cancelled" || e?.code === "busy") return; // superseded
      if (task.kind === "interactive") { setError(t("editor.traceFailed", { msg: errMsg(e) })); setErrorKind("trace"); }
    }
  }, [projectId, filename, imageId, applyResultToConfigs, scheduleUserConfig, t]);

  // One queue per image: interactive traces preempt background pre-warm.
  const queueRef = useRef(null);
  useEffect(() => {
    const q = createTraceQueue({ execute: executeTask, onBusyChange: setBusy });
    queueRef.current = q;
    return () => {
      q.dispose();
      queueRef.current = null;
    };
  }, [executeTask]);

  // Clear any pending user-config snapshot on unmount.
  useEffect(() => () => {
    if (configTimerRef.current) clearTimeout(configTimerRef.current);
  }, []);

  // Any param change (preset select, slider, smart tuning) enqueues an
  // interactive trace. The queue debounces (merges slider drags), drops queued
  // background work, and the engine cancels whatever is in flight for this
  // image when the request lands — interactive traces always win the CPU.
  // (Declared after the queue effect so the queue exists when it runs.)
  useEffect(() => {
    if (!imageUrl) return;
    queueRef.current?.enqueueInteractive(params);
  }, [params, imageUrl, imageId]);

  // Pre-warm in the background after each interactive trace settles: the
  // system presets plus any user-saved configs whose trace is not cached yet
  // (so selecting them is instant and their thumbnails fill in without a
  // reload). Tasks run serially only while idle and are dropped the moment
  // the user interacts. `planned` (per image + settle) stops the configs
  // re-renders caused by our own thumbnail updates from re-adding
  // already-completed tasks; a new settle resets it, so tasks dropped by user
  // interaction are retried later.
  const prewarmPlanRef = useRef({ imageId: null, result: null, sigs: new Set() });
  useEffect(() => {
    if (!result || busy) return;
    if (prewarmPlanRef.current.imageId !== imageId || prewarmPlanRef.current.result !== result) {
      prewarmPlanRef.current = { imageId, result, sigs: new Set() };
    }
    const sigs = prewarmPlanRef.current.sigs;
    const plan = (p) => {
      const sig = stableStringify(p);
      if (sigs.has(sig)) return;
      sigs.add(sig);
      queueRef.current?.enqueueBackground(p);
    };
    for (const key of PREWARM_ORDER) plan({ ...DEFAULT_PARAMS, ...presetFullParams(key) });
    for (const c of configs) {
      if (c.saved && !c.result) plan({ ...DEFAULT_PARAMS, ...c.params });
    }
  }, [result, busy, imageId, configs]);

  const onParamsChange = useCallback((updater) => {
    setParams(updater);
    setActiveConfigId(null);
  }, []);

  const onConfigSelect = useCallback((config) => {
    setActiveConfigId(config.id);
    // Remember the selected system preset: the next image opened from the
    // list (same or different) starts with it.
    if (config.isDefault) {
      const key = config.id.startsWith("default-") ? config.id.slice("default-".length) : null;
      if (key && MODE_PRESETS[key]) saveLastPreset(key);
    }
    // Normalize: configs saved before newer params fields exist.
    const next = { ...DEFAULT_PARAMS, ...config.params };
    setParams(prev =>
      stableStringify(prev) === stableStringify(next) ? prev : next
    );
  }, []);

  /**
   * Smart tuning: search for the best parameters *within the preset family
   * the user is currently working in*. The param change triggers the normal
   * auto-trace, so the preview updates for free.
   */
  const runSmart = useCallback(async () => {
    if (!imageUrl || smartBusy) return;
    setSmartBusy(true);
    setSmartInfo(null);
    setError(null);
    setErrorKind(null);
    try {
      const source = prepareSource(projectId, filename);
      const family = familyFromParams(params);
      const decision = await smartSource(source, family);
      setSmartInfo(decision);
      if (decision?.params) {
        const next = { ...DEFAULT_PARAMS, ...decision.params };
        setParams(prev =>
          stableStringify(prev) === stableStringify(next) ? prev : next
        );
        setActiveConfigId(null);
      }
    } catch (e) {
      setError(t("editor.smartFailed", { msg: errMsg(e) }));
      setErrorKind("smart");
    } finally {
      setSmartBusy(false);
    }
  }, [imageUrl, smartBusy, projectId, filename, params, t]);

  /**
   * Apply the preset-default result (from the smart comparison) instead of the
   * smart winner, so the user can fall back to the known-good preset if the
   * tuned result doesn't look right.
   */
  const applyPresetResult = useCallback(() => {
    const preset = smartInfo?.preset;
    if (!preset?.params) return;
    const next = { ...DEFAULT_PARAMS, ...preset.params };
    setParams(prev =>
      stableStringify(prev) === stableStringify(next) ? prev : next
    );
    setActiveConfigId(null);
  }, [smartInfo]);

  // Clear the smart banner when switching to a different image.
  useEffect(() => { setSmartInfo(null); }, [imageId]);

  // Backfill saved-config thumbnails once the initial trace has warmed the
  // cache: the restore effect above may have run while the cache was still
  // cold (e.g. a first visit to this image), leaving them as placeholders.
  useEffect(() => {
    if (!result || busy) return;
    const pending = configsRef.current.filter(c => c.saved && !c.result);
    if (!pending.length) return;
    let cancelled = false;
    (async () => {
      for (const c of pending) {
        const res = await getCachedTrace(imageId, c.params).catch(() => null);
        if (cancelled || !res) continue;
        applyResultToConfigs(c.params, res);
      }
    })();
    return () => { cancelled = true; };
  }, [result, busy, applyResultToConfigs, imageId]);

  const onConfigDelete = useCallback((id) => {
    const target = configsRef.current.find(c => c.id === id);
    if (target?.saved) deleteSavedConfig(id).catch(() => {});
    setConfigs(prev => prev.filter(c => c.id !== id));
    setActiveConfigId(prev => (prev === id ? null : prev));
  }, []);

  /** Rename a user config; re-persist if it is saved. */
  const onConfigRename = useCallback((id, label) => {
    const target = configsRef.current.find(c => c.id === id);
    if (!target || target.label === label) return;
    setConfigs(prev => prev.map(c => (c.id === id ? { ...c, label } : c)));
    if (target.saved) saveConfig({ ...target, label }).catch(() => {});
  }, []);

  /** Toggle persistence for a user config (save / unsave). */
  const onConfigSave = useCallback((config) => {
    if (config.saved) {
      deleteSavedConfig(config.id).catch(() => {});
    } else {
      saveConfig(config).catch(() => {});
    }
    setConfigs(prev => prev.map(c => (c.id === config.id ? { ...c, saved: !c.saved } : c)));
  }, []);

  const onDownload = useCallback(() => {
    if (result?.svg) {
      downloadSvg(result.svg, filename.replace(/\.\w+$/, "") + ".svg");
    }
  }, [result, filename]);

  // The actually-applied smart winner (may be the preset baseline when no
  // variation clearly beat it), and whether it *is* the preset.
  const smartWinner = smartInfo?.candidates?.find(
    c => stableStringify(c.params) === stableStringify(smartInfo?.params)
  );
  const smartIsPreset = !!(
    smartInfo?.preset &&
    stableStringify(smartInfo.preset.params) === stableStringify(smartInfo?.params)
  );

  // Candidate labels arrive as i18n keys: preset family ids (translated via
  // presetLabel) or "photo" / "flat" / "presetDefault" (candidate.* keys).
  const candidateLabel = (label) =>
    MODE_PRESETS[label] ? presetLabel(label) : t(`candidate.${label}`);

  // Human-readable smart decision, translated from the backend's reasonKey
  // ("scoped" = search was restricted to one preset family, incl. the preset
  // baseline). Legacy results carry a pre-built `reason` string instead.
  const smartReason = smartInfo?.reasonKey
    ? t(smartInfo.preset ? "smart.reasonScoped" : "smart.reasonAuto", {
        colors: smartInfo.stats?.effectiveColors ?? 0,
        colorfulness: Math.round(smartInfo.stats?.colorfulness ?? 0),
        bimodality: (smartInfo.stats?.bimodality ?? 0).toFixed(2),
        texture: Math.round((smartInfo.stats?.texture ?? 0) * 100),
        family: presetLabel(smartInfo.family),
        n: smartInfo.candidates?.length ?? 0,
        winner: candidateLabel(smartWinner?.label || smartInfo.family),
        fidelity: (smartWinner?.fidelity ?? 0).toFixed(2),
        complexity: (smartWinner?.complexity ?? 0).toFixed(2),
        shapes: smartWinner?.shapeCount ?? 0,
      })
    : smartInfo?.reason;

  return (
    <div className={`editor-page${maximized ? " maximized" : ""}${advancedMode ? " advanced" : ""}`}>
      <AppHeader
        title={filename}
        onBack={() => navigate(`/project/${encodeURIComponent(projectId)}`)}
      >
        <button onClick={onDownload} disabled={!result?.svg}>{t("editor.downloadSvg")}</button>
        {result?.width > 0 && (
          <span className="page-meta">{result.width} x {result.height} px</span>
        )}
        <button
          className="icon-btn"
          onClick={toggleMaximize}
          title={maximized ? t("editor.restore") : t("editor.maximize")}
          aria-label={maximized ? t("editor.restore") : t("editor.maximize")}
        >
          {maximized ? <IconRestore /> : <IconMaximize />}
        </button>
      </AppHeader>

      <div className="editor-main">
        <aside className="editor-sidebar">
          <ParameterPanel
            params={params}
            onChange={onParamsChange}
            onSmart={runSmart}
            smartBusy={smartBusy}
            onOpenPreprocess={() => setPreprocessOpen(true)}
            activeFamilyLabel={presetLabel(familyFromParams(params))}
          />
        </aside>

        <div className="editor-content">
          {advancedMode && result?.svg ? (
            <div className="preview-viewport" ref={advViewportRef}>
              <AdvancedSvgEditor
                ref={advEditorRef}
                svgString={result.svg}
                onComplete={(editedSvg) => {
                  // Non-retrace result update: keep any layers the user
                  // snapshotted from the advanced editor.
                  skipLayerResetRef.current = true;
                  setResult(prev => prev ? { ...prev, svg: editedSvg } : prev);
                  setAdvancedMode(false);
                }}
                onCancel={() => setAdvancedMode(false)}
                onSaveAsLayer={saveAdvancedAsLayer}
              />
              <div className="preview-controls">
                <button onClick={() => advEditorRef.current?.zoomIn()} title={t("editor.zoomIn")} aria-label={t("editor.zoomIn")}>
                  <IconZoomIn />
                </button>
                <button onClick={() => advEditorRef.current?.zoomOut()} title={t("editor.zoomOut")} aria-label={t("editor.zoomOut")}>
                  <IconZoomOut />
                </button>
                <button onClick={() => advEditorRef.current?.zoomFit()} title={t("editor.fit")} aria-label={t("editor.fit")}>
                  <IconFit />
                </button>
              </div>
            </div>
          ) : (
            <CompareViewer
              key={imageId}
              leftUrl={imageUrl}
              right={result?.svg ? { svg: result.svg } : null}
              contentSize={{ w: result?.width || srcSize.w || 0, h: result?.height || srcSize.h || 0 }}
              onImageLoad={(w, h) => setSrcSize({ w, h })}
              compareMode={compareMode}
              onCompareModeChange={setCompareMode}
              busy={busy}
              leftLabel={t("editor.original")}
              rightLabel={t("editor.vector")}
              busyLabel={t("processing")}
              placeholder={t("editor.clickToStart")}
              leftPlaceholder={t("editor.loading")}
              selectMode={tool === "select"}
              svgContainerRef={svgContainerRef}
              extraControls={<>
                <button className={tool === "select" ? "active" : ""} onClick={() => setTool(tool === "select" ? "none" : "select")} title={tool === "select" ? t("editor.disableSelection") : t("editor.selectPatches")} aria-label={tool === "select" ? t("editor.disableSelection") : t("editor.selectPatches")}>
                  <IconSelect />
                </button>
                <button className={advancedMode ? "active" : ""} onClick={() => { if (!advancedMode && result?.svg) { setAdvancedMode(true); setTool("none"); } }} title={t("adv.enter")} aria-label={t("adv.enter")} disabled={!result?.svg || advancedMode}>
                  <IconAdvanced />
                </button>
              </>}
            />
          )}


          {error && (
            <div className="error preview-error">
              {error}
              <button className="error-retry" onClick={() => {
                if (errorKind === "smart") runSmart();
                else if (errorKind === "load") setReloadTick(x => x + 1);
                else queueRef.current?.enqueueInteractive(params);
              }}>{t("retry")}</button>
            </div>
          )}
        </div>

        <aside className="editor-layers">
          <LayerPanel
            layers={[...(result?.layers || []), ...savedLayers]}
            strategyInfo={result?.strategyInfo}
            profileMs={result?.profileMs}
            // In advanced mode the preview stage is unmounted, so the panel's
            // snapshot button delegates to the advanced editor's canvas.
            onSaveAsLayer={advancedMode ? () => advEditorRef.current?.saveAsLayer() : saveAsLayer}
            canSaveAsLayer={!!result?.svg}
          />
        </aside>
      </div>

      {smartInfo?.params && (
        <div className="smart-banner">
          <span className="smart-banner-icon">✨</span>
          <span className="smart-banner-text">
            {t("editor.smartLabel")}<strong>{candidateLabel(smartWinner?.label || smartInfo.family)}</strong>
            {smartReason ? ` — ${smartReason}` : ""}
          </span>
          {smartInfo.preset && (
            <span
              className="smart-banner-compare"
              title={t("editor.presetMetricsTitle")}
            >
              {t("editor.presetMetrics", {
                f: smartInfo.preset.fidelity.toFixed(2),
                c: smartInfo.preset.complexity.toFixed(2),
              })}
            </span>
          )}
          {smartInfo.preset && !smartIsPreset && (
            <button className="smart-banner-preset" onClick={applyPresetResult}>
              {t("editor.usePresetResult")}
            </button>
          )}
          <button className="smart-banner-close" onClick={() => setSmartInfo(null)} aria-label={t("close")}>×</button>
        </div>
      )}
      {!advancedMode && (
        <ConfigStrip
          configs={configs}
          activeConfigId={activeConfigId}
          onConfigSelect={onConfigSelect}
          onConfigSave={onConfigSave}
          onConfigDelete={onConfigDelete}
          onConfigRename={onConfigRename}
        />
      )}

      <PreprocessModal
        open={preprocessOpen}
        onClose={() => setPreprocessOpen(false)}
        projectId={projectId}
        filename={filename}
        imageUrl={imageUrl}
        onSaved={(newFilename) => navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(newFilename)}`)}
      />
    </div>
  );
}

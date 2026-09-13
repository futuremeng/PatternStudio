import React, { useEffect, useLayoutEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, prepareSource, traceSource, smartSource, newTraceRequestId, downloadSvg, MODE_PRESETS, DEFAULT_PARAMS, presetFullParams, familyFromParams } from "../api.js";
import { getCachedTrace, setCachedTrace, stableStringify } from "../traceCache.js";
import { createTraceQueue } from "../traceQueue.js";
import { loadSavedConfigs, saveConfig, deleteSavedConfig } from "../configStore.js";
import ParameterPanel from "../components/ParameterPanel.jsx";
import LayerPanel from "../components/LayerPanel.jsx";
import ConfigStrip from "../components/ConfigStrip.jsx";
import AppHeader from "../components/AppHeader.jsx";

// Wait this long after a trace settles before snapshotting a user config (ms)
const CONFIG_SETTLE_MS = 1500;
const MAX_USER_CONFIGS = 12;
// Pre-warm: presets to pre-generate in the background after the initial trace,
// in priority order. The currently-active preset is already cached and skipped.
const PREWARM_ORDER = ["color", "lineArt", "bw", "gradient", "lowPoly"];

/** Default configs from MODE_PRESETS (full param sets, ready to trace). */
function buildDefaultConfigs() {
  return Object.entries(MODE_PRESETS).map(([key, preset]) => ({
    id: `default-${key}`,
    label: preset.label,
    params: presetFullParams(key),
    isDefault: true,
  }));
}

// --- Preview view-control icons (inline SVG, 16px) ---
const IconMaximize = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M8 3H5a2 2 0 0 0-2 2v3M16 3h3a2 2 0 0 1 2 2v3M8 21H5a2 2 0 0 1-2-2v-3M16 21h3a2 2 0 0 0 2-2v-3" />
  </svg>
);
const IconRestore = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3" />
  </svg>
);
const IconSelect = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
    <path d="M4 4l0 14 4.5-4.5 3 6.5 2.5-1.2-3-6.3 6 0z" />
  </svg>
);
const IconFitWidth = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M10 8l-4 4 4 4M14 8l4 4-4 4" />
  </svg>
);
const IconCompare = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
    <rect x="11" y="4" width="2" height="16" rx="1" />
    <path d="M8 12L3 7v10z" />
    <path d="M16 12l5-5v10z" />
  </svg>
);
const IconZoomIn = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 5v14M5 12h14" />
  </svg>
);
const IconZoomOut = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M5 12h14" />
  </svg>
);
const IconFit = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <rect x="4" y="4" width="16" height="16" rx="2" />
  </svg>
);

export default function ImageEditorPage() {
  const { projectId, filename } = useParams();
  const navigate = useNavigate();
  const [imageUrl, setImageUrl] = useState(null);
  const [params, setParams] = useState(DEFAULT_PARAMS);
  const [result, setResult] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [configs, setConfigs] = useState(buildDefaultConfigs);
  const [activeConfigId, setActiveConfigId] = useState("default-color");
  const [compareMode, setCompareMode] = useState("side"); // "side" | "overlay"
  const [overlayPos, setOverlayPos] = useState(50); // divider position, percent
  const [smartInfo, setSmartInfo] = useState(null); // last Smart Preset decision
  const [smartBusy, setSmartBusy] = useState(false);
  const configTimerRef = useRef(null);
  const userConfigSeq = useRef(0);
  const configsRef = useRef(configs);
  const overlayRef = useRef(null);
  const overlayDragging = useRef(false);
  const imageId = `${projectId}/${filename}`;
  const imageIdRef = useRef(imageId);
  useEffect(() => { imageIdRef.current = imageId; }, [imageId]);

  // --- Preview view controls: maximize, zoom, fit ---
  const [maximized, setMaximized] = useState(false);
  const [zoom, setZoom] = useState(1);
  const [actualSize, setActualSize] = useState(false);
  const [tool, setTool] = useState("none");
  const viewportRef = useRef(null);
  const [viewportSize, setViewportSize] = useState({ w: 0, h: 0 });
  // Natural size of the source image (known as soon as it loads, before the
  // first trace finishes). Used to size the vector placeholder and enable
  // zoom/fit controls from the very first paint.
  const [srcSize, setSrcSize] = useState({ w: 0, h: 0 });

  // --- Patch selection & dragging ---
  const svgContainerRef = useRef(null);
  const pathTransformsRef = useRef(new Map()); // pathIdx → { dx, dy } in SVG coords
  const dragRef = useRef(null); // { idx, startClientX, startClientY, origDx, origDy }
  const justDraggedRef = useRef(false);
  const selectedPathIdxRef = useRef(null);

  // Track the visible preview size so we can fit the image at zoom = 1 and
  // scale it precisely for the zoom / fit-width / fit controls. Measured in a
  // layout effect so the first paint already has a correct size.
  useLayoutEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      setViewportSize({ w: r.width, h: r.height });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  // Reset the view when switching to a different image.
  useEffect(() => { setZoom(1); setActualSize(false); }, [imageId]);

  const toggleMaximize = useCallback(() => setMaximized(m => !m), []);
  const zoomIn = useCallback(() => { setActualSize(false); setZoom(z => Math.min(8, z * 1.25)); }, []);
  const zoomOut = useCallback(() => { setActualSize(false); setZoom(z => Math.max(0.1, z / 1.25)); }, []);
  const fitScreen = useCallback(() => { setActualSize(false); setZoom(1); }, []);

  // Ctrl/Cmd + wheel zoom on the preview viewport; plain wheel scrolls natively.
  const panDragRef = useRef(null); // { startX, startY, scrollLeft, scrollTop }
  useEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const onWheel = (e) => {
      if (!e.ctrlKey && !e.metaKey) return; // let plain wheel scroll
      e.preventDefault();
      const factor = e.deltaY < 0 ? 1.1 : 1 / 1.1;
      setZoom(z => Math.min(8, Math.max(0.1, z * factor)));
    };
    el.addEventListener("wheel", onWheel, { passive: false });

    // Drag-to-pan when zoomed in
    const onPointerDown = (e) => {
      if (e.button !== 0) return;
      if (el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight) return;
      panDragRef.current = {
        startX: e.clientX, startY: e.clientY,
        scrollLeft: el.scrollLeft, scrollTop: el.scrollTop,
      };
      el.style.cursor = "grabbing";
      el.setPointerCapture(e.pointerId);
    };
    const onPointerMove = (e) => {
      const d = panDragRef.current;
      if (!d) return;
      el.scrollLeft = d.scrollLeft - (e.clientX - d.startX);
      el.scrollTop = d.scrollTop - (e.clientY - d.startY);
    };
    const onPointerUp = (e) => {
      if (!panDragRef.current) return;
      panDragRef.current = null;
      el.style.cursor = "";
      el.releasePointerCapture(e.pointerId);
    };
    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("pointermove", onPointerMove);
    el.addEventListener("pointerup", onPointerUp);
    el.addEventListener("pointercancel", onPointerUp);

    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("pointermove", onPointerMove);
      el.removeEventListener("pointerup", onPointerUp);
      el.removeEventListener("pointercancel", onPointerUp);
    };
  }, []);

  // Fit metrics derived from the image's natural size and the viewport.
  // Fall back to the source image's natural size so the layout (and the
  // vector placeholder) is stable before the first trace result arrives.
  const imgW = result?.width || srcSize.w || 0;
  const imgH = result?.height || srcSize.h || 0;
  const isSide = compareMode === "side";
  const PAD = 16; // matches .preview-stage padding
  const GAP = 16; // matches .before-after gap (fixed, does not scale)
  const availW = Math.max(0, viewportSize.w - PAD * 2);
  const availH = Math.max(0, viewportSize.h - PAD * 2);
  // Scale at which the content exactly fills the viewport (zoom = 1).
  const fitScale = imgW > 0 && imgH > 0 && availW > 0 && availH > 0
    ? (isSide
        ? Math.min(Math.max(0, availW - GAP) / (imgW * 2), availH / imgH)
        : Math.min(availW / imgW, availH / imgH))
    : 0;
  // Scale at which the content exactly fills the viewport width (for 全宽).
  const widthScale = imgW > 0 && availW > 0
    ? (isSide ? Math.max(0, availW - GAP) / (imgW * 2) : availW / imgW)
    : 0;
  const hasDims = fitScale > 0;
  const ds = hasDims ? fitScale * zoom : 0;
  const boxStyle = hasDims ? { width: imgW * ds, height: imgH * ds } : undefined;
  const imgStyle = hasDims ? boxStyle : { maxHeight: "70vh" };

  const fitWidth = useCallback(() => {
    if (fitScale > 0 && widthScale > 0) setZoom(widthScale / fitScale);
  }, [fitScale, widthScale]);

  useEffect(() => { configsRef.current = configs; }, [configs]);

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
        if (!cancelled) setError(e.message);
      }
    })();
    return () => { cancelled = true; };
  }, [projectId, filename]);

  // Overlay compare: move the divider to a pointer x position (percent).
  const updateOverlayPos = useCallback((clientX) => {
    const el = overlayRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0) return;
    const pos = ((clientX - rect.left) / rect.width) * 100;
    setOverlayPos(Math.min(100, Math.max(0, pos)));
  }, []);

  useEffect(() => {
    const onMove = (e) => { if (overlayDragging.current) updateOverlayPos(e.clientX); };
    const onUp = () => { overlayDragging.current = false; };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [updateOverlayPos]);

  // --- Patch selection & dragging helpers ---

  const getSvgAndPaths = useCallback(() => {
    const container = svgContainerRef.current;
    if (!container) return null;
    const svg = container.querySelector("svg");
    if (!svg) return null;
    return { svg, paths: Array.from(svg.querySelectorAll("path")) };
  }, []);

  const clearPatchSelection = useCallback(() => {
    selectedPathIdxRef.current = null;
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

  const highlightPath = useCallback((idx) => {
    const res = getSvgAndPaths();
    if (!res) return;
    res.paths.forEach((p, i) => {
      if (i === idx) {
        p.style.stroke = "#6c5ce7";
        p.style.strokeWidth = "2";
        p.style.cursor = "move";
      } else {
        p.style.stroke = "";
        p.style.strokeWidth = "";
        p.style.cursor = "pointer";
      }
    });
  }, [getSvgAndPaths]);

  const renderControlPoints = useCallback((idx) => {
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
    if (idx == null || !paths[idx]) return;
    const path = paths[idx];
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
      c.setAttribute("fill", "#6c5ce7"); c.setAttribute("stroke", "#fff");
      c.setAttribute("stroke-width", (r * 0.35).toFixed(1));
      c.classList.add("patch-cp"); c.style.pointerEvents = "none";
      svg.appendChild(c);
    }
  }, [getSvgAndPaths]);

  // Reset patch selection when the SVG changes (new trace result).
  useEffect(() => {
    selectedPathIdxRef.current = null;
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
    if (selectedPathIdxRef.current != null && paths[selectedPathIdxRef.current]) {
      highlightPath(selectedPathIdxRef.current);
      renderControlPoints(selectedPathIdxRef.current);
    }

    const onPathPointerDown = (e) => {
      if (e.button !== 0) return;
      const idx = paths.indexOf(e.target);
      if (idx === -1) return;
      e.preventDefault();
      e.stopPropagation();
      selectedPathIdxRef.current = idx;
      highlightPath(idx);
      renderControlPoints(idx);
      const t = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
      dragRef.current = { idx, sx: e.clientX, sy: e.clientY, ox: t.dx, oy: t.dy };

      const onMove = (me) => {
        const d = dragRef.current;
        if (!d) return;
        const rect = svg.getBoundingClientRect();
        const vb = svg.viewBox.baseVal;
        if (!vb || vb.width === 0 || rect.width === 0) return;
        const dx = d.ox + (me.clientX - d.sx) * (vb.width / rect.width);
        const dy = d.oy + (me.clientY - d.sy) * (vb.height / rect.height);
        pathTransformsRef.current.set(d.idx, { dx, dy });
        paths[d.idx].setAttribute("transform", `translate(${dx}, ${dy})`);
        renderControlPoints(d.idx);
      };
      const onUp = () => {
        if (dragRef.current) {
          justDraggedRef.current = true;
          setTimeout(() => { justDraggedRef.current = false; }, 0);
        }
        dragRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    const onSvgClick = (e) => {
      if (justDraggedRef.current) return;
      if (e.target === svg) clearPatchSelection();
    };
    const onKeyDown = (e) => {
      if (e.key === "Escape") { clearPatchSelection(); return; }
      if ((e.key === "Delete" || e.key === "Backspace") && selectedPathIdxRef.current != null) {
        e.preventDefault();
        const idx = selectedPathIdxRef.current;
        const current = getSvgAndPaths();
        if (current && current.paths[idx]) {
          current.paths[idx].remove();
          pathTransformsRef.current.delete(idx);
        }
        selectedPathIdxRef.current = null;
        const after = getSvgAndPaths();
        if (after) after.paths.forEach(p => { p.style.stroke = ""; p.style.strokeWidth = ""; p.style.cursor = ""; });
        if (after) after.svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
      }
    };

    paths.forEach(p => {
      p.style.pointerEvents = "auto";
      p.style.cursor = "pointer";
      p.style.touchAction = "none";
      p.addEventListener("pointerdown", onPathPointerDown);
    });
    svg.addEventListener("click", onSvgClick);
    window.addEventListener("keydown", onKeyDown);

    return () => {
      paths.forEach(p => {
        p.style.pointerEvents = "";
        p.style.cursor = "";
        p.style.touchAction = "";
        p.removeEventListener("pointerdown", onPathPointerDown);
      });
      svg.removeEventListener("click", onSvgClick);
      window.removeEventListener("keydown", onKeyDown);
      dragRef.current = null;
    };
  }, [tool, result?.svg, compareMode, getSvgAndPaths, highlightPath, renderControlPoints, clearPatchSelection]);

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
        label: `自定义 ${userConfigSeq.current}`,
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
  }, []);

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
        scheduleUserConfig(task.params, res);
      }
    } catch (e) {
      if (imageIdRef.current !== imageIdNow) return;
      if (e?.code === "cancelled" || e?.code === "busy") return; // superseded
      if (task.kind === "interactive") setError(`Trace 失败: ${e.message}`);
    }
  }, [projectId, filename, imageId, applyResultToConfigs, scheduleUserConfig]);

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
      setError(`智能调参失败: ${e.message}`);
    } finally {
      setSmartBusy(false);
    }
  }, [imageUrl, smartBusy, projectId, filename, params]);

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

  return (
    <div className={`editor-page${maximized ? " maximized" : ""}`}>
      <AppHeader
        title={filename}
        onBack={() => navigate(`/project/${encodeURIComponent(projectId)}`)}
      >
        <button onClick={onDownload} disabled={!result?.svg}>下载 SVG</button>
        {result?.width > 0 && (
          <span className="page-meta">{result.width} x {result.height} px</span>
        )}
        <button
          className="icon-btn"
          onClick={toggleMaximize}
          title={maximized ? "还原" : "最大化"}
          aria-label={maximized ? "还原" : "最大化"}
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
            activeFamilyLabel={MODE_PRESETS[familyFromParams(params)]?.label}
          />
        </aside>

        <div className="editor-content">
          <div className="preview-viewport" ref={viewportRef}>
            <div className="preview-stage">
              {compareMode === "side" ? (
                <div className="before-after">
                  <div className="ba-panel">
                    <span className="label">原图</span>
                    {imageUrl ? (
                      <img
                        src={imageUrl}
                        alt="original"
                        style={imgStyle}
                        onLoad={(e) => setSrcSize({ w: e.currentTarget.naturalWidth, h: e.currentTarget.naturalHeight })}
                      />
                    ) : <div className="placeholder">加载中</div>}
                  </div>
                  <div className="ba-panel">
                    <span className="label">矢量</span>
                    {busy && result?.svg && <span className="busy-badge">处理中...</span>}
                    {result?.svg ? (
                      <div className={`svg-frame${tool === "select" ? " select-mode" : ""}`} ref={svgContainerRef} style={boxStyle} dangerouslySetInnerHTML={{ __html: result.svg }} />
                    ) : (
                      <div className="placeholder" style={boxStyle}>{busy ? "处理中..." : "点击下方配置开始"}</div>
                    )}
                  </div>
                </div>
              ) : (
                <div className="overlay-wrap">
                  {busy && result?.svg && <span className="busy-badge">处理中...</span>}
                  {imageUrl && result?.svg ? (
                    <div
                      className="overlay-compare"
                      ref={overlayRef}
                      style={boxStyle}
                      onPointerDown={(e) => {
                        e.preventDefault();
                        overlayDragging.current = true;
                        updateOverlayPos(e.clientX);
                      }}
                    >
                      <img src={imageUrl} alt="original" />
                      <div className="overlay-top" style={{ clipPath: `inset(0 0 0 ${overlayPos}%)` }}>
                        <div className={`overlay-svg${tool === "select" ? " select-mode" : ""}`} ref={svgContainerRef} dangerouslySetInnerHTML={{ __html: result.svg }} />
                      </div>
                      <span className="overlay-tag overlay-tag-left">原图</span>
                      <span className="overlay-tag overlay-tag-right">矢量</span>
                      <div className="overlay-divider" style={{ left: `${overlayPos}%` }}>
                        <div className="overlay-handle">‹›</div>
                      </div>
                    </div>
                  ) : (
                    <div className="placeholder" style={boxStyle}>{busy ? "处理中..." : "点击下方配置开始"}</div>
                  )}
                </div>
              )}
            </div>
          </div>

          <div className="preview-controls">
            <button className={tool === "select" ? "active" : ""} onClick={() => setTool(tool === "select" ? "none" : "select")} title={tool === "select" ? "Disable Selection" : "Select Patches"} aria-label={tool === "select" ? "Disable Selection" : "Select Patches"}>
              <IconSelect />
            </button>
            <button onClick={() => setCompareMode(compareMode === "side" ? "overlay" : "side")} title={compareMode === "side" ? "切换为叠加" : "切换为对照"} aria-label="切换对照模式">
              <IconCompare />
            </button>
            <button onClick={zoomIn} title="放大" aria-label="放大">
              <IconZoomIn />
            </button>
            <button onClick={zoomOut} title="缩小" aria-label="缩小">
              <IconZoomOut />
            </button>
            <button
              onClick={() => {
                if (actualSize) {
                  setActualSize(false);
                  setZoom(1);
                } else if (fitScale > 0) {
                  setActualSize(true);
                  setZoom(1 / fitScale);
                }
              }}
              title={actualSize ? "铺满" : "1:1"}
              aria-label={actualSize ? "铺满" : "1:1"}
            >
              {actualSize ? <IconFit /> : <span style={{ fontSize: "0.65rem", fontWeight: 700 }}>1:1</span>}
            </button>
          </div>

          {error && (
            <div className="error preview-error">
              {error}
              <button className="error-retry" onClick={() => queueRef.current?.enqueueInteractive(params)}>重试</button>
            </div>
          )}
        </div>

        <aside className="editor-layers">
          <LayerPanel layers={result?.layers || []} strategyInfo={result?.strategyInfo} profileMs={result?.profileMs} />
        </aside>
      </div>

      {smartInfo?.params && (
        <div className="smart-banner">
          <span className="smart-banner-icon">✨</span>
          <span className="smart-banner-text">
            智能调参：<strong>{smartWinner?.label || smartInfo.family}</strong>
            {smartInfo.reason ? ` — ${smartInfo.reason}` : ""}
          </span>
          {smartInfo.preset && (
            <span
              className="smart-banner-compare"
              title="预设默认结果的指标（对照基准）"
            >
              预设 保真 {smartInfo.preset.fidelity.toFixed(2)} · 复杂度{" "}
              {smartInfo.preset.complexity.toFixed(2)}
            </span>
          )}
          {smartInfo.preset && !smartIsPreset && (
            <button className="smart-banner-preset" onClick={applyPresetResult}>
              使用预设结果
            </button>
          )}
          <button className="smart-banner-close" onClick={() => setSmartInfo(null)} aria-label="关闭">×</button>
        </div>
      )}
      <ConfigStrip
        configs={configs}
        activeConfigId={activeConfigId}
        onConfigSelect={onConfigSelect}
        onConfigSave={onConfigSave}
        onConfigDelete={onConfigDelete}
        onConfigRename={onConfigRename}
      />
    </div>
  );
}

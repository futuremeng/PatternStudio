import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, getProjectLineage, prepareSource, vectorizeSource, layerSource, cancelTrace, newTraceRequestId, downloadSvg, MODE_PRESETS, DEFAULT_PARAMS, presetFullParams, familyFromParams, splitTraceParams, mergeTraceParams } from "../api.js";
import { getCachedTrace, setCachedTrace, getCachedVectorize, setCachedVectorize, stableStringify } from "../traceCache.js";
import { createTraceQueue } from "../traceQueue.js";
import { loadSavedConfigs, saveConfig, deleteSavedConfig } from "../configStore.js";
import { loadLastPreset, saveLastPreset, errMsg, buildDefaultConfigs, dominantFill } from "../editorUtils.js";
import { usePatchEditing } from "../hooks/usePatchEditing.js";
import { useSmartPreset } from "../hooks/useSmartPreset.js";
import ParameterPanel from "../components/ParameterPanel.jsx";
import LayerPanel from "../components/LayerPanel.jsx";
import ConfigStrip from "../components/ConfigStrip.jsx";
import AppHeader from "../components/AppHeader.jsx";
import AdvancedSvgEditor from "../components/AdvancedSvgEditor.jsx";
import CompareViewer, { useViewportControls } from "../components/CompareViewer.jsx";
import PreprocessModal from "../components/PreprocessModal.jsx";
import SmartBanner from "../components/SmartBanner.jsx";
import { IconMaximize, IconRestore, IconSelect, IconZoomIn, IconZoomOut, IconFit, IconAdvanced } from "../components/icons.jsx";
import { useI18n } from "../i18n/index.jsx";

// Wait this long after a trace settles before snapshotting a user config (ms)
const CONFIG_SETTLE_MS = 1500;
const MAX_USER_CONFIGS = 12;
// Pre-warm: presets to pre-generate in the background after the initial trace,
// in priority order. The currently-active preset is already cached and skipped.
const PREWARM_ORDER = ["color", "lineArt", "bw", "gradient", "lowPoly"];

export default function ImageEditorPage() {
  const { t, presetLabel, locale } = useI18n();
  const { projectId, filename } = useParams();
  const navigate = useNavigate();
  const [imageUrl, setImageUrl] = useState(null);
  // Vectorization params (left panel): changing any of these re-vectorizes.
  const [params, setParams] = useState(() => {
    const key = loadLastPreset();
    return splitTraceParams(key ? presetFullParams(key) : DEFAULT_PARAMS).trace;
  });
  // Layering/output params (layer panel): applied manually via "Process".
  const [layerParams, setLayerParams] = useState(() => {
    const key = loadLastPreset();
    return splitTraceParams(key ? presetFullParams(key) : DEFAULT_PARAMS).layer;
  });
  const [result, setResult] = useState(null);
  // Signature of the vectorize params that produced the current preview doc
  // (null = nothing vectorized yet). Drives the layer panel's "pending" state.
  const [lastVectorizeSig, setLastVectorizeSig] = useState(null);
  // Signature of the full params of the current processed (layered) result.
  const [lastProcessSig, setLastProcessSig] = useState(null);
  // Layers the user has snapshotted from the edited preview (see saveAsLayer).
  // Kept separate from result.layers so a re-trace can reset them cleanly.
  const [savedLayers, setSavedLayers] = useState([]);
  const [busy, setBusy] = useState(false);
  // True only while a layering ("Process") task is in flight — drives the
  // layer panel's button, decoupled from the vectorization busy flag.
  const [processBusy, setProcessBusy] = useState(false);
  const [error, setError] = useState(null);
  // What the error banner is about, so its retry button re-runs the right thing.
  const [errorKind, setErrorKind] = useState(null); // "vectorize" | "layer" | "smart" | "load"
  // Bumped to re-run the image-load effect (retry after a load failure).
  const [reloadTick, setReloadTick] = useState(0);
  const [configs, setConfigs] = useState(() => buildDefaultConfigs(presetLabel));
  const [activeConfigId, setActiveConfigId] = useState(() => {
    const key = loadLastPreset();
    return key ? `default-${key}` : "default-color";
  });
  const [compareMode, setCompareMode] = useState("side"); // "side" | "overlay"
  // Digital-art enhancement modal (Real-ESRGAN anime): iterative enhancement
  // with per-iteration thumbnails; saving writes a new image into the project
  // folder and switches the editor to it.
  const [preprocessOpen, setPreprocessOpen] = useState(false);
  // Lineage entry of the current image (null when it is an original): lets
  // the preprocess modal trace new iterations back to the root original.
  const [currentLineage, setCurrentLineage] = useState(null);
  // True once the lineage lookup for the current image has settled (success
  // or failure). The auto-trace waits for it: whether the current file lives
  // in the generated subfolder is derived from the lineage, so tracing
  // before the lookup settles could read the wrong path.
  const [lineageLoaded, setLineageLoaded] = useState(false);
  useEffect(() => {
    let cancelled = false;
    setCurrentLineage(null);
    setLineageLoaded(false);
    getProjectLineage(projectId)
      .then((entries) => {
        if (cancelled) return;
        const match = entries.find(e => e.confirmedFile === filename || e.file === filename);
        setCurrentLineage(match || null);
      })
      .catch(() => {})
      .finally(() => { if (!cancelled) setLineageLoaded(true); });
    return () => { cancelled = true; };
  }, [projectId, filename]);
  // The current file is an auto-generated iteration (stored under
  // .patternstudio/generated/) when the lineage entry's `file` (the map key)
  // is this filename and it was not confirmed *as this same filename*
  // (legacy inferred entries key by the top-level name, so file ===
  // confirmedFile there).
  const isGenerated = !!(
    lineageLoaded &&
    currentLineage &&
    currentLineage.file === filename &&
    currentLineage.confirmedFile !== filename
  );
  const configTimerRef = useRef(null);
  const userConfigSeq = useRef(0);
  const configsRef = useRef(configs);
  const imageId = `${projectId}/${filename}`;
  const imageIdRef = useRef(imageId);
  useEffect(() => { imageIdRef.current = imageId; }, [imageId]);
  // Ref mirror so the auto-vectorize effect can read the latest layering
  // params without depending on them (layering changes must NOT re-vectorize).
  const layerParamsRef = useRef(layerParams);
  useEffect(() => { layerParamsRef.current = layerParams; }, [layerParams]);
  // Which image the current `imageUrl` belongs to (set by the load effect).
  // The auto-trace effect waits for the *current* image's URL so the stale
  // URL from the previous image can't kick off a throwaway trace that the
  // fresh URL's trace would immediately preempt on the engine.
  const urlImageIdRef = useRef(null);
  // The previous { projectId, filename }, so a switch can cancel the old
  // image's in-flight engine work (interactive trace or orphaned pre-warm).
  const prevSourceRef = useRef(null);
  // imageId -> the source token that image actually traced with (its path may
  // point at the generated subfolder), so switching away can cancel the
  // in-flight engine work with the exact same key.
  const sourceByImageRef = useRef(new Map());

  // --- Preview view controls: maximize ---
  const [maximized, setMaximized] = useState(false);
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

  // The canvas every separated layer and every layer export is emitted on: the
  // size the artwork was traced at. Separating a layer takes a subset of the
  // shapes, it does not crop the artwork, so the canvas must never shrink to
  // their bounding box.
  const canvasSize = useMemo(
    () => ({ w: result?.width || srcSize.w || 0, h: result?.height || srcSize.h || 0 }),
    [result?.width, result?.height, srcSize.w, srcSize.h]
  );

  // --- Patch selection & dragging (see usePatchEditing) ---
  const snapshotEditedLayer = useCallback((snap) => {
    setSavedLayers((prev) => [
      ...prev,
      {
        name: t("layer.editedLayer", { n: prev.length + 1 }),
        role: "edited",
        shapeCount: snap.shapeCount,
        dominantColor: snap.dominantColor,
        previewSvg: snap.previewSvg,
        profile: null,
      },
    ]);
  }, [t]);
  const { tool, setTool, svgContainerRef, saveAsLayer } = usePatchEditing({
    result,
    compareMode,
    canvasSize,
    onSnapshotLayer: snapshotEditedLayer,
  });

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
  // `result` must be cleared too: otherwise the previous image's vector is
  // shown for the new one (next to the "processing" badge) until its trace
  // lands, which reads as a frozen/stuck editor.
  useEffect(() => {
    setAdvancedMode(false);
    setResult(null);
    setLastVectorizeSig(null);
    setLastProcessSig(null);
  }, [imageId]);

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
  // `imageUrl` is cleared first so a switched-to image shows a loading
  // placeholder instead of the previous image's bitmap.
  useEffect(() => {
    let cancelled = false;
    setSrcSize({ w: 0, h: 0 });
    setImageUrl(null);
    (async () => {
      try {
        const url = await getImageUrl(projectId, filename);
        if (cancelled) return;
        urlImageIdRef.current = `${projectId}/${filename}`;
        setImageUrl(url);
      } catch (e) {
        if (!cancelled) { setError(errMsg(e)); setErrorKind("load"); }
      }
    })();
    return () => { cancelled = true; };
  }, [projectId, filename, reloadTick]);

  // Switching images: cancel the previous image's in-flight engine work (its
  // interactive trace or an orphaned pre-warm) so it stops competing with the
  // new image's trace for CPU. Fire-and-forget; a no-op when nothing runs.
  useEffect(() => {
    const prev = prevSourceRef.current;
    prevSourceRef.current = { projectId, filename };
    if (prev && (prev.projectId !== projectId || prev.filename !== filename)) {
      // Cancel with the source the previous image actually traced with; fall
      // back to the default top-level source when it never traced.
      const prevSource = sourceByImageRef.current.get(`${prev.projectId}/${prev.filename}`)
        || prepareSource(prev.projectId, prev.filename);
      cancelTrace(prevSource);
    }
  }, [projectId, filename]);

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

  // --- Smart Preset tuning (see useSmartPreset) ---
  // Normalize + apply a full param set, deselecting the active config (the
  // smart winner / preset fallback produces params no existing config owns).
  // Only vectorization runs automatically; layering stays manual.
  const applyParams = useCallback((nextParams) => {
    const next = { ...DEFAULT_PARAMS, ...nextParams };
    const { trace, layer } = splitTraceParams(next);
    setParams(prev => stableStringify(prev) === stableStringify(trace) ? prev : trace);
    setLayerParams(prev => stableStringify(prev) === stableStringify(layer) ? prev : layer);
    setActiveConfigId(null);
    // Explicit enqueue: if the vectorize part is unchanged the auto effect
    // below won't fire, but the preview should still refresh.
    queueRef.current?.enqueueInteractive("vectorize", next);
  }, []);
  const mergedParams = mergeTraceParams(params, layerParams);
  const { smartInfo, setSmartInfo, smartBusy, runSmart, applyPresetResult } = useSmartPreset({
    imageUrl, params: mergedParams, projectId, filename, imageId, isGenerated,
    t, applyParams, setError, setErrorKind,
  });

  /**
   * Write a trace result back to any config whose params match, so the
   * strip thumbnails (presets / saved configs) update without a reload.
   * No-op when no config matches (plain user tweaks).
   */
  const applyResultToConfigs = useCallback((traceParams, res) => {
    const sig = stableStringify(traceParams);
    setConfigs(prev =>
      prev.some(c => stableStringify(c.params) === sig)
        ? prev.map(c => stableStringify(c.params) === sig ? { ...c, result: res } : c)
        : prev
    );
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
   * Run one queue task (cache-first) and apply the result.
   * - stage "vectorize": image → vectorized document preview (unprocessed).
   * - stage "layer": cached document → layered SVG (manual processing).
   * - stage "chain": vectorize + layer in sequence (background pre-warm).
   * Interactive tasks update the main preview; background tasks only refresh
   * matching config thumbnails. Superseded tasks (cancelled / busy) are a
   * silent no-op — a newer task already owns the result.
   */
  const executeTask = useCallback(async (task) => {
    const imageIdNow = imageId;
    const source = prepareSource(projectId, filename, { generated: isGenerated });
    sourceByImageRef.current.set(imageIdNow, source);
    const isInteractive = task.kind === "interactive";
    const priority = isInteractive ? "interactive" : "background";
    const { trace: tracePart } = splitTraceParams(task.params);
    const traceSig = stableStringify(tracePart);

    // Vectorization stage: cache-first on the vectorize-params signature.
    const runVectorize = async () => {
      let res = await getCachedVectorize(imageIdNow, tracePart).catch(() => null);
      if (imageIdRef.current !== imageIdNow) return null; // image switched
      if (res) return res;
      try {
        res = await vectorizeSource(source, task.params, {
          requestId: newTraceRequestId(),
          priority,
        });
      } catch (e) {
        if (imageIdRef.current !== imageIdNow) return null;
        if (e?.code === "cancelled" || e?.code === "busy") return null; // superseded
        if (isInteractive) { setError(t("editor.traceFailed", { msg: errMsg(e) })); setErrorKind("vectorize"); }
        return "error";
      }
      if (imageIdRef.current !== imageIdNow) return null;
      setCachedVectorize(imageIdNow, tracePart, res).catch(() => {});
      return res;
    };

    // Layering stage: cache-first on the full params signature.
    const runLayer = async () => {
      let res = await getCachedTrace(imageIdNow, task.params).catch(() => null);
      if (imageIdRef.current !== imageIdNow) return null; // image switched
      if (res) return res;
      try {
        res = await layerSource(source, task.params, {
          requestId: newTraceRequestId(),
          priority,
        });
      } catch (e) {
        if (imageIdRef.current !== imageIdNow) return null;
        if (e?.code === "cancelled" || e?.code === "busy") return null; // superseded
        if (isInteractive) { setError(t("editor.traceFailed", { msg: errMsg(e) })); setErrorKind("layer"); }
        return "error";
      }
      if (imageIdRef.current !== imageIdNow) return null;
      setCachedTrace(imageIdNow, task.params, res).catch(() => {});
      return res;
    };

    if (task.stage === "vectorize") {
      const res = await runVectorize();
      if (!res || res === "error") return;
      if (isInteractive) {
        setResult({ ...res, processed: false });
        setLastVectorizeSig(traceSig);
        setLastProcessSig(null);
        setError(null);
        setErrorKind(null);
        scheduleUserConfig(task.params, { ...res, processed: false });
      }
      return;
    }

    // "layer" (manual processing) or "chain" (background pre-warm).
    if (task.stage === "chain") {
      const vres = await runVectorize();
      if (!vres || vres === "error") return;
    }
    const lres = await runLayer();
    if (!lres || lres === "error") return;
    applyResultToConfigs(task.params, lres);
    if (isInteractive) {
      setResult({ ...lres, processed: true });
      setLastVectorizeSig(traceSig);
      setLastProcessSig(stableStringify(task.params));
      setError(null);
      setErrorKind(null);
      scheduleUserConfig(task.params, { ...lres, processed: true });
    }
  }, [projectId, filename, isGenerated, imageId, applyResultToConfigs, scheduleUserConfig, t]);

  // The queue reports which stage (if any) an interactive task is running.
  // `busy` (main preview badge + pre-warm/backfill gating) covers any
  // interactive work; `processBusy` (layer panel button) covers layering only.
  const handleBusyChange = useCallback((stage) => {
    setBusy(stage != null);
    setProcessBusy(stage === "layer");
  }, []);

  // One queue per image: interactive traces preempt background pre-warm.
  const queueRef = useRef(null);
  useEffect(() => {
    const q = createTraceQueue({ execute: executeTask, onBusyChange: handleBusyChange });
    queueRef.current = q;
    return () => {
      q.dispose();
      queueRef.current = null;
    };
  }, [executeTask, handleBusyChange]);

  // Clear any pending user-config snapshot on unmount.
  useEffect(() => () => {
    if (configTimerRef.current) clearTimeout(configTimerRef.current);
  }, []);

  // Any vectorization param change (preset select, slider, smart tuning)
  // enqueues an interactive vectorize task. The queue debounces (merges
  // slider drags), drops queued background work, and the engine cancels
  // whatever is in flight for this image when the request lands — interactive
  // work always wins the CPU. Layering param changes are deliberately NOT
  // enqueued: layering is applied manually via the layer panel's "Process".
  // (Declared after the queue effect so the queue exists when it runs.)
  useEffect(() => {
    // Only vectorize once the URL actually belongs to the current image (see
    // urlImageIdRef): on a switch, the stale URL from the previous image is
    // still around for a frame and must not start a throwaway trace. Also
    // wait for the lineage lookup (lineageLoaded): the trace source path
    // depends on whether this file lives in the generated subfolder.
    if (!imageUrl || urlImageIdRef.current !== imageId || !lineageLoaded) return;
    queueRef.current?.enqueueInteractive("vectorize", mergeTraceParams(params, layerParamsRef.current));
  }, [params, imageUrl, imageId, lineageLoaded]);

  // Layering param changes: no automatic processing — the user applies them
  // via the "Process" button. Deselect the active config (the full param
  // combination no longer matches it).
  const onLayerParamsChange = useCallback((updater) => {
    setLayerParams(updater);
    setActiveConfigId(null);
  }, []);

  // Manual processing: apply the current layering strategy / output style to
  // the vectorized document (cheap — the document is cached).
  const runLayerProcessing = useCallback(() => {
    queueRef.current?.enqueueInteractive("layer", mergeTraceParams(params, layerParams));
  }, [params, layerParams]);

  /**
   * Abort a running layering (auto layering re-fits every shape and can take a
   * long time). The engine cancels the in-flight request for this image at its
   * next shape boundary and answers 409; the queue treats "cancelled" as a
   * silent no-op, so the previous result stays on screen.
   */
  const cancelLayerProcessing = useCallback(() => {
    const source = sourceByImageRef.current.get(imageId)
      || prepareSource(projectId, filename, { generated: isGenerated });
    cancelTrace(source);
  }, [projectId, filename, isGenerated, imageId]);

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
    const { trace, layer } = splitTraceParams(next);
    setParams(prev =>
      stableStringify(prev) === stableStringify(trace) ? prev : trace
    );
    setLayerParams(prev =>
      stableStringify(prev) === stableStringify(layer) ? prev : layer
    );
    // Preset/config selection auto-runs vectorization only; layering stays
    // manual. Explicit enqueue: if the vectorize part is unchanged the auto
    // effect won't fire, but the preview should still refresh.
    queueRef.current?.enqueueInteractive("vectorize", next);
  }, []);

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

  return (
    <div className={`editor-page${maximized ? " maximized" : ""}${advancedMode ? " advanced" : ""}`}>
      <AppHeader
        title={filename}
        // Back to the lineage tree page of this image's root original (the
        // page the editor is reached from); for an original image that is
        // its own tree page.
        onBack={() => navigate(`/project/${encodeURIComponent(projectId)}/tree/${encodeURIComponent(currentLineage?.root || filename)}`)}
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
            activeFamilyLabel={presetLabel(familyFromParams(mergedParams))}
          />
        </aside>

        <div className="editor-content">
          {advancedMode && result?.svg ? (
            <div className="preview-viewport" ref={advViewportRef}>
              <AdvancedSvgEditor
                ref={advEditorRef}
                svgString={result.svg}
                canvasSize={canvasSize}
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
              contentSize={canvasSize}
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
                else if (errorKind === "layer") queueRef.current?.enqueueInteractive("layer", mergeTraceParams(params, layerParams));
                else queueRef.current?.enqueueInteractive("vectorize", mergeTraceParams(params, layerParams));
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
            layerParams={layerParams}
            onLayerParamsChange={onLayerParamsChange}
            onProcess={runLayerProcessing}
            onCancel={cancelLayerProcessing}
            processBusy={processBusy}
            processPending={lastVectorizeSig != null && stableStringify(mergedParams) !== lastProcessSig}
            processed={!!result?.processed}
            canvasSize={canvasSize}
          />
        </aside>
      </div>

      <SmartBanner
        smartInfo={smartInfo}
        t={t}
        presetLabel={presetLabel}
        onApplyPreset={applyPresetResult}
        onClose={() => setSmartInfo(null)}
      />
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
        currentLineage={currentLineage}
        isGenerated={isGenerated}
        onSaved={(newFilename) => navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(newFilename)}`)}
      />
    </div>
  );
}

import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, prepareSource, traceSource, downloadSvg, MODE_PRESETS, DEFAULT_PARAMS, presetFullParams } from "../api.js";
import { getCachedTrace, setCachedTrace, stableStringify } from "../traceCache.js";
import { loadSavedConfigs, saveConfig, deleteSavedConfig } from "../configStore.js";
import ParameterPanel from "../components/ParameterPanel.jsx";
import LayerPanel from "../components/LayerPanel.jsx";
import ConfigStrip from "../components/ConfigStrip.jsx";
import AppHeader from "../components/AppHeader.jsx";

// Debounce for auto-trace after param changes (ms)
const TRACE_DEBOUNCE_MS = 300;
// Wait this long after a trace settles before snapshotting a user config (ms)
const CONFIG_SETTLE_MS = 1500;
const MAX_USER_CONFIGS = 12;
// Pre-warm: presets to pre-generate in the background after the initial trace,
// in priority order. The currently-active preset is already cached and skipped.
const PREWARM_ORDER = ["color", "lineArt", "bw", "gradient", "lowPoly"];
// If the user interacted (param change / preset switch) within this window, the
// pre-warm yields so interactive traces always win the CPU.
const PREWARM_IDLE_MS = 2000;

/** Default configs from MODE_PRESETS (full param sets, ready to trace). */
function buildDefaultConfigs() {
  return Object.entries(MODE_PRESETS).map(([key, preset]) => ({
    id: `default-${key}`,
    label: preset.label,
    params: presetFullParams(key),
    isDefault: true,
  }));
}

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
  const [retryTick, setRetryTick] = useState(0);
  const [compareMode, setCompareMode] = useState("side"); // "side" | "overlay"
  const [overlayPos, setOverlayPos] = useState(50); // divider position, percent
  const traceSeq = useRef(0);
  const configTimerRef = useRef(null);
  const userConfigSeq = useRef(0);
  const configsRef = useRef(configs);
  const overlayRef = useRef(null);
  const overlayDragging = useRef(false);
  const busyRef = useRef(false);
  const resultRef = useRef(null);
  const lastUserActiveRef = useRef(0);
  const imageId = `${projectId}/${filename}`;

  useEffect(() => { configsRef.current = configs; }, [configs]);
  useEffect(() => { busyRef.current = busy; }, [busy]);
  useEffect(() => { resultRef.current = result; }, [result]);

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

  // Auto-trace: any param change (debounced) regenerates immediately.
  // Cache-first, so reverting a slider hits IndexedDB and is instant.
  useEffect(() => {
    if (!imageUrl) return;
    const seq = ++traceSeq.current;
    const timer = setTimeout(async () => {
      const cached = await getCachedTrace(imageId, params).catch(() => null);
      if (seq !== traceSeq.current) return;
      if (cached) {
        setResult(cached);
        setError(null);
        scheduleUserConfig(params, cached);
        return;
      }
      setBusy(true);
      setError(null);
      try {
        // Resolve the source once (path on Tauri, project+file on web) and let
        // the engine reuse its cached session/segmentation across tweaks.
        const source = prepareSource(projectId, filename);
        const res = await traceSource(source, params);
        if (seq !== traceSeq.current) return;
        setResult(res);
        setCachedTrace(imageId, params, res).catch(() => {});
        scheduleUserConfig(params, res);
      } catch (e) {
        if (seq === traceSeq.current) setError(`Trace 失败: ${e.message}`);
      } finally {
        if (seq === traceSeq.current) setBusy(false);
      }
    }, TRACE_DEBOUNCE_MS);
    return () => {
      clearTimeout(timer);
      if (configTimerRef.current) {
        clearTimeout(configTimerRef.current);
        configTimerRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [params, imageUrl, retryTick, imageId, projectId, filename]);

  // Pre-warm the remaining presets in the background after the initial trace
  // settles, so switching between presets is instant. Runs once per image,
  // serially, and yields whenever the user is active or a trace is in flight.
  // It only writes the cache — it never touches the displayed result.
  useEffect(() => {
    if (!imageUrl) return;
    let cancelled = false;
    const isUserActive = () => Date.now() - lastUserActiveRef.current < PREWARM_IDLE_MS;
    (async () => {
      // Wait for the initial trace to settle (a result exists and we're idle).
      while (!cancelled && (!resultRef.current || busyRef.current)) {
        await sleep(200);
      }
      if (cancelled) return;
      for (const key of PREWARM_ORDER) {
        if (cancelled) return;
        // Yield while the user is active or a trace is in flight.
        while (!cancelled && (busyRef.current || isUserActive())) {
          await sleep(400);
        }
        if (cancelled) return;
        // Same params onConfigSelect would set, so the cache key matches.
        const p = { ...DEFAULT_PARAMS, ...presetFullParams(key) };
        const cached = await getCachedTrace(imageId, p).catch(() => null);
        if (cancelled) return;
        if (cached) continue; // already warm (e.g. the active preset)
        try {
          const source = prepareSource(projectId, filename);
          const res = await traceSource(source, p);
          if (cancelled) return;
          await setCachedTrace(imageId, p, res).catch(() => {});
        } catch (e) {
          // Pre-warm failure is non-fatal; skip this preset.
        }
      }
    })();
    return () => { cancelled = true; };
  }, [imageUrl, imageId, projectId, filename]);

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

  const onParamsChange = useCallback((updater) => {
    lastUserActiveRef.current = Date.now();
    setParams(updater);
    setActiveConfigId(null);
  }, []);

  const onConfigSelect = useCallback((config) => {
    lastUserActiveRef.current = Date.now();
    setActiveConfigId(config.id);
    // Normalize: configs saved before newer params fields exist.
    const next = { ...DEFAULT_PARAMS, ...config.params };
    setParams(prev =>
      stableStringify(prev) === stableStringify(next) ? prev : next
    );
  }, []);

  const onConfigDelete = useCallback((id) => {
    const target = configsRef.current.find(c => c.id === id);
    if (target?.saved) deleteSavedConfig(id).catch(() => {});
    setConfigs(prev => prev.filter(c => c.id !== id));
    setActiveConfigId(prev => (prev === id ? null : prev));
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
    <div className="editor-page">
      <AppHeader
        title={filename}
        onBack={() => navigate(`/project/${encodeURIComponent(projectId)}`)}
      >
        <div className="seg-control">
          <button className={compareMode === "side" ? "active" : ""} onClick={() => setCompareMode("side")}>对照</button>
          <button className={compareMode === "overlay" ? "active" : ""} onClick={() => setCompareMode("overlay")}>叠加</button>
        </div>
        <button onClick={onDownload} disabled={!result?.svg}>下载 SVG</button>
      </AppHeader>

      <div className="editor-main">
        <aside className="editor-sidebar">
          <ParameterPanel params={params} onChange={onParamsChange} />
        </aside>

        <div className="editor-content">
          {compareMode === "side" ? (
            <div className="before-after">
              <div className="ba-panel">
                <span className="label">原图</span>
                {imageUrl ? <img src={imageUrl} alt="original" /> : <div className="placeholder">加载中</div>}
              </div>
              <div className="ba-panel">
                <span className="label">矢量</span>
                {busy && result?.svg && <span className="busy-badge">处理中...</span>}
                {result?.svg ? (
                  <div className="svg-frame" dangerouslySetInnerHTML={{ __html: result.svg }} />
                ) : (
                  <div className="placeholder">{busy ? "处理中..." : "点击下方配置开始"}</div>
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
                  onPointerDown={(e) => {
                    e.preventDefault();
                    overlayDragging.current = true;
                    updateOverlayPos(e.clientX);
                  }}
                >
                  <img src={imageUrl} alt="original" />
                  <div className="overlay-top" style={{ clipPath: `inset(0 0 0 ${overlayPos}%)` }}>
                    <div className="overlay-svg" dangerouslySetInnerHTML={{ __html: result.svg }} />
                  </div>
                  <span className="overlay-tag overlay-tag-left">原图</span>
                  <span className="overlay-tag overlay-tag-right">矢量</span>
                  <div className="overlay-divider" style={{ left: `${overlayPos}%` }}>
                    <div className="overlay-handle">‹›</div>
                  </div>
                </div>
              ) : (
                <div className="placeholder">{busy ? "处理中..." : "点击下方配置开始"}</div>
              )}
            </div>
          )}
          {error && (
            <div className="error" style={{ padding: "0 1rem" }}>
              {error}
              <button className="error-retry" onClick={() => setRetryTick(t => t + 1)}>重试</button>
            </div>
          )}
        </div>

        <aside className="editor-layers">
          <LayerPanel layers={result?.layers || []} strategyInfo={result?.strategyInfo} profileMs={result?.profileMs} />
        </aside>
      </div>

      <ConfigStrip
        configs={configs}
        activeConfigId={activeConfigId}
        onConfigSelect={onConfigSelect}
        onConfigSave={onConfigSave}
        onConfigDelete={onConfigDelete}
      />
    </div>
  );
}

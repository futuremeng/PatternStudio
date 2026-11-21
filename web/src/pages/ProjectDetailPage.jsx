import React, { useEffect, useState, useCallback, useRef, useMemo } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, startBatchTrace, presetFullParams, MODE_PRESETS, invalidateImageListCache } from "../api.js";
import { getCachedTrace, setCachedTrace } from "../traceCache.js";
import AppHeader from "../components/AppHeader.jsx";
import ContextMenu, { useContextMenu } from "../components/ContextMenu.jsx";
import { useProject } from "../hooks/useProject.js";
import { useI18n } from "../i18n/index.jsx";

/**
 * Cache marker for smart-batch results, per preset family. The winning params
 * differ per image, so smart results are keyed by this stable marker (for the
 * "already done?" pre-check) and additionally under the winning params (so
 * the editor's smart flow hits the cache).
 */
const smartCacheKey = (family) => ({ __smart: family });

/**
 * Crop an SVG to one of its quarters *of the cover square* by rewriting the
 * viewBox. The cover square is the same square thumbnail the left panel's
 * raster uses (object-fit: cover on a square box): side = max(w, h),
 * centered on the image. Normalizing all three panels to that one square
 * first is what keeps them aligned — left half + top-right quarter +
 * bottom-right quarter tile the same square at the same scale.
 * quarter: "tr" = top-right, "br" = bottom-right.
 * Falls back to the width/height attributes when no viewBox exists.
 */
function cropSvgQuarter(svg, quarter) {
  let x = 0, y = 0, w, h, vbAttr = null;
  const vb = svg.match(/viewBox="([^"]+)"/);
  if (vb) {
    [x, y, w, h] = vb[1].split(/[\s,]+/).map(Number);
    if ([x, y, w, h].some(n => !Number.isFinite(n))) return svg;
    vbAttr = vb[0];
  } else {
    const wm = svg.match(/width="([\d.]+)(px)?"/);
    const hm = svg.match(/height="([\d.]+)(px)?"/);
    if (!wm || !hm) return svg;
    w = parseFloat(wm[1]);
    h = parseFloat(hm[1]);
  }
  // Same "thumbnail" as the raster: cover-fit to a centered square.
  const side = Math.max(w, h);
  const sx = x + (w - side) / 2;
  const sy = y + (h - side) / 2;
  const viewBox = `viewBox="${sx + side / 2} ${quarter === "tr" ? sy : sy + side / 2} ${side / 2} ${side / 2}"`;
  return vbAttr ? svg.replace(vbAttr, viewBox) : svg.replace(/<svg/, `<svg ${viewBox}`);
}

/**
 * A single image card that lazy-loads its thumbnail only when scrolled near
 * the viewport. In Tauri mode each thumbnail is a full-resolution image read
 * over IPC, so eagerly loading a 100-image project would stall the UI and
 * balloon memory; IntersectionObserver keeps only the visible cards loaded.
 */
function ImageCard({ projectId, filename, preview, showSplit, onClick, onContextMenu }) {
  const { t, presetLabel } = useI18n();
  const [url, setUrl] = useState(null);
  const cardRef = useRef(null);

  // Lazy-load: fetch the image once the card nears the viewport.
  useEffect(() => {
    if (url) return;
    const el = cardRef.current;
    if (!el) return;
    let cancelled = false;
    const io = new IntersectionObserver(
      async (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        io.disconnect();
        try {
          const u = await getImageUrl(projectId, filename);
          if (!cancelled) setUrl(u);
        } catch {
          /* ignore load failures; the placeholder stays visible */
        }
      },
      { rootMargin: "400px 0px" }
    );
    io.observe(el);
    return () => {
      cancelled = true;
      io.disconnect();
    };
  }, [projectId, filename, url]);

  // Release the object URL when it changes or the card unmounts.
  useEffect(() => {
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  }, [url]);

  return (
    <div className="image-card" ref={cardRef} onClick={onClick} onContextMenu={onContextMenu}>
      {showSplit && preview?.result ? (
        // Single-preset batch result (bw / gradient / lowPoly / smart), shown full on the right.
        <div className="img-wrap split">
          <div className="split-left">
            {url && <img src={url} alt={filename} />}
          </div>
          <div className="split-right">
            <div className="split-panel">
              <div className="svg-mini" dangerouslySetInnerHTML={{ __html: preview.result }} />
              {preview.label && <div className="mini-label">{presetLabel(preview.label)}</div>}
            </div>
          </div>
        </div>
      ) : showSplit && preview ? (
        <div className="img-wrap split">
          <div className="split-left">
            {url && <img src={url} alt={filename} />}
          </div>
          <div className="split-right">
            <div className="split-panel">
              {preview.line
                ? <div className="svg-mini" dangerouslySetInnerHTML={{ __html: cropSvgQuarter(preview.line, "tr") }} />
                : <div className="mini-empty">{t("detail.lineArt")}</div>}
            </div>
            <div className="split-panel">
              {preview.color
                ? <div className="svg-mini" dangerouslySetInnerHTML={{ __html: cropSvgQuarter(preview.color, "br") }} />
                : <div className="mini-empty">{t("detail.colorArt")}</div>}
            </div>
          </div>
        </div>
      ) : (
        <div className="img-wrap">
          {url && <img src={url} alt={filename} />}
        </div>
      )}
      <div className="name">{filename}</div>
    </div>
  );
}

export default function ProjectDetailPage() {
  const { t, presetLabel } = useI18n();
  const { projectId } = useParams();
  const navigate = useNavigate();
  const { menu, open, close } = useContextMenu();
  // Bumped when files land in the project folder (drag & drop import) so the
  // grid re-lists it.
  const [refreshKey, setRefreshKey] = useState(0);
  const { project, error } = useProject(projectId, refreshKey);
  // Batch: the user picks a preset family (what output they want), then
  // chooses fixed params or smart tuning (best params within that family).
  const [batchPreset, setBatchPreset] = useState("lineArt");
  const [batchSmart, setBatchSmart] = useState(false);
  // Display mode for the grid: off = original thumbnail, on = three-image split.
  const [showSplit, setShowSplit] = useState(false);
  const [batchStatus, setBatchStatus] = useState(null);
  const [batchProgress, setBatchProgress] = useState(null);
  const [batchFailed, setBatchFailed] = useState(0);
  // Cached line/color SVGs per image, for the split preview on processed cards
  const [previews, setPreviews] = useState({});
  const batchHandleRef = useRef(null);
  const cancelRef = useRef(false);

  // Originals only: top-level images that are not a confirmed derived image
  // (those live in the lineage tree page). The grid and batch processing
  // both target this set.
  const originals = useMemo(() => {
    if (!project?.images?.length) return [];
    const confirmedFiles = new Set((project.lineage || []).map(e => e.confirmedFile).filter(Boolean));
    return project.images.filter(n => !confirmedFiles.has(n));
  }, [project]);

  // Load cached line/color results (fixed preset params or smart results)
  // for the split preview.
  useEffect(() => {
    if (!originals.length) return;
    let cancelled = false;
    // The cache check is one storage read per image; doing them one after
    // another made a large project's previews land only after a long wait.
    // Read them in small concurrent batches and fill the grid as each lands.
    const CHUNK = 8;
    (async () => {
      for (let i = 0; i < originals.length; i += CHUNK) {
        const chunk = originals.slice(i, i + CHUNK);
        const found = await Promise.all(chunk.map(async (name) => {
          const imageId = `${projectId}/${name}`;
          const [lineFixed, lineSmart, colorFixed, colorSmart] = await Promise.all([
            getCachedTrace(imageId, presetFullParams("lineArt")).catch(() => null),
            getCachedTrace(imageId, smartCacheKey("lineArt")).catch(() => null),
            getCachedTrace(imageId, presetFullParams("color")).catch(() => null),
            getCachedTrace(imageId, smartCacheKey("color")).catch(() => null),
          ]);
          const line = (lineFixed || lineSmart)?.svg || null;
          const color = (colorFixed || colorSmart)?.svg || null;
          return line || color ? { name, preview: { line, color } } : null;
        }));
        if (cancelled) return;
        const patch = {};
        for (const r of found) if (r) patch[r.name] = r.preview;
        if (Object.keys(patch).length) setPreviews(prev => ({ ...prev, ...patch }));
      }
    })();
    return () => { cancelled = true; };
  }, [originals, projectId]);

  const onBatchTrace = useCallback(async () => {
    if (!project) return;
    // One pass per image with the selected preset. Smart mode: the engine
    // analyzes each image and trial-traces parameter variations *within* the
    // chosen family, using the winner per image.
    const family = batchPreset;
    const jobs = [{
      pkey: family,
      family,
      smart: batchSmart,
      params: presetFullParams(family),
    }];
    // Cache-first: only send images without a same-params result to the backend.
    // Originals only — confirmed derived images live in the lineage tree page
    // and are never batch-processed here.
    const plan = [];
    for (const job of jobs) {
      const key = job.smart ? smartCacheKey(job.family) : job.params;
      // One concurrent pass over the whole project: checking images one by one
      // delayed the start of the batch by a round trip per image.
      const cached = await Promise.all(
        originals.map(name =>
          getCachedTrace(`${projectId}/${name}`, key).then(r => !!r).catch(() => false)
        )
      );
      const missing = originals.filter((_, i) => !cached[i]);
      if (missing.length) plan.push({ ...job, missing });
    }
    const total = plan.reduce((s, j) => s + j.missing.length, 0);
    if (total === 0) {
      setBatchStatus("done");
      setBatchFailed(0);
      setBatchProgress({ current: originals.length, total: originals.length, file: "" });
      return;
    }
    cancelRef.current = false;
    setBatchStatus("running");
    setBatchFailed(0);
    setBatchProgress({ current: 0, total, file: "" });

    let doneCount = 0;
    let failedCount = 0;
    for (const job of plan) {
      const handle = startBatchTrace(projectId, job.params, job.missing, !!job.smart, job.smart ? job.family : undefined);
      batchHandleRef.current = handle;
      handle.setOnEvent((event) => {
        if (event.type === "processing") {
          setBatchProgress({ current: doneCount, total, file: event.file });
        } else if (event.type === "done") {
          doneCount += 1;
          setBatchProgress(p => ({ ...p, current: doneCount }));
          const res = {
            svg: event.svg,
            width: event.width,
            height: event.height,
            shapeCount: event.shapeCount ?? event.shape_count,
            layerCount: (event.layers || []).length,
            layers: event.layers || [],
          };
          if (job.smart) {
            // Persist under the stable per-family marker (pre-check) and
            // under the winning params (so the editor's smart flow hits the cache).
            setCachedTrace(`${projectId}/${event.file}`, smartCacheKey(job.family), res).catch(() => {});
            if (event.smart?.params) {
              setCachedTrace(`${projectId}/${event.file}`, event.smart.params, res).catch(() => {});
            }
          } else {
            // Persist the generated result so the editor page hits the cache
            setCachedTrace(`${projectId}/${event.file}`, job.params, res).catch(() => {});
          }
          // Preview slot: lineArt / color feed the split panels; the other
          // presets take the full right panel with a label.
          const slot = job.family === "lineArt" ? "line" : job.family === "color" ? "color" : "result";
          const update = { [slot]: event.svg };
          if (slot === "result") update.label = job.family;
          setPreviews(prev => ({
            ...prev,
            [event.file]: { ...(prev[event.file] || {}), ...update },
          }));
        } else if (event.type === "error") {
          doneCount += 1;
          failedCount += 1;
          setBatchFailed(f => f + 1);
          setBatchProgress(p => ({ ...p, current: doneCount }));
        } else if (event.type === "cancelled") {
          setBatchStatus("cancelled");
        }
      });
      await handle.start;
      await handle.done;
      handle.close();
      if (cancelRef.current) break;
    }
    if (!cancelRef.current) {
      setBatchStatus("done");
      setBatchFailed(failedCount);
      setBatchProgress({ current: total, total, file: "" });
    }
  }, [project, originals, projectId, batchPreset, batchSmart]);

  const onCancelBatch = useCallback(() => {
    cancelRef.current = true;
    setBatchStatus("cancelled");
    batchHandleRef.current?.cancel();
  }, []);

  // Stop the batch when the page unmounts mid-run
  useEffect(() => () => {
    cancelRef.current = true;
    batchHandleRef.current?.cancel();
  }, []);

  // A drag & drop import (see App) re-lists the folder.
  useEffect(() => {
    const onChanged = (e) => {
      if (e.detail?.projectId !== projectId) return;
      invalidateImageListCache(projectId);
      setRefreshKey(k => k + 1);
    };
    window.addEventListener("ps:images-changed", onChanged);
    return () => window.removeEventListener("ps:images-changed", onChanged);
  }, [projectId]);

  if (!project) {
    // Keep the header while loading and on failure: without it this page was
    // a dead end (deleted project / unreachable server) with no way back.
    return (
      <div className="project-detail-page">
        <AppHeader title={projectId.split("/").pop()} onBack={() => navigate("/")} />
        <div className="placeholder">
          {error ? (
            <div className="error">
              {error}
              <button className="error-retry" onClick={() => setRefreshKey(k => k + 1)}>{t("retry")}</button>
            </div>
          ) : t("loading")}
        </div>
      </div>
    );
  }

  const allDone = batchStatus === "done" && batchFailed === 0;
  const batchLabel =
    batchStatus === "running" ? t("detail.batchRunning")
    : allDone ? t("detail.batchDone")
    : batchStatus === "cancelled" ? t("detail.batchResume")
    : batchStatus === "done" ? t("detail.batchRetryFailed")
    : batchSmart ? t("detail.batchSmart", { label: presetLabel(batchPreset) })
    : t("detail.batchFixed", { label: presetLabel(batchPreset) });

  return (
    <div className="project-detail-page">
      <AppHeader
        title={project.name}
        meta={t("imageCount", { n: originals.length })}
        onBack={() => navigate("/")}
      >
        <label className="split-toggle" title={t("detail.splitTitle")}>
          <span className="split-toggle-label">{t("detail.split")}</span>
          <button
            type="button"
            className={`switch ${showSplit ? "on" : ""}`}
            role="switch"
            aria-checked={showSplit}
            aria-label={t("detail.split")}
            onClick={() => setShowSplit(v => !v)}
          >
            <span className="knob" />
          </button>
        </label>
        <select
          className="batch-preset-select"
          value={batchPreset}
          onChange={e => { setBatchPreset(e.target.value); setBatchStatus(null); }}
          disabled={batchStatus === "running"}
          title={t("detail.presetTitle")}
        >
          {Object.entries(MODE_PRESETS).map(([key]) => (
            <option key={key} value={key}>{presetLabel(key)}</option>
          ))}
        </select>
        <div className="seg-control" title={t("detail.smartTitle")}>
          <button className={!batchSmart ? "active" : ""} onClick={() => { setBatchSmart(false); setBatchStatus(null); }}>{t("detail.fixed")}</button>
          <button className={batchSmart ? "active seg-wide" : "seg-wide"} onClick={() => { setBatchSmart(true); setBatchStatus(null); }}>{t("detail.smart")}</button>
        </div>
        {batchStatus === "running" && (
          <button className="danger" onClick={onCancelBatch}>{t("detail.abort")}</button>
        )}
        <button className="primary" onClick={onBatchTrace} disabled={batchStatus === "running" || allDone}>
          {batchLabel}
        </button>
      </AppHeader>

      <div className="image-grid-scroll">
        <div className="image-grid">
          {originals.map(filename => (
            <ImageCard
              key={filename}
              projectId={projectId}
              filename={filename}
              preview={previews[filename]}
              showSplit={showSplit}
              onClick={() => navigate(`/project/${encodeURIComponent(projectId)}/tree/${encodeURIComponent(filename)}`)}
              onContextMenu={(e) =>
                open(e, [
                  { label: t("ctx.open"), onSelect: () => navigate(`/project/${encodeURIComponent(projectId)}/tree/${encodeURIComponent(filename)}`) },
                ])
              }
            />
          ))}
        </div>
      </div>
      <ContextMenu menu={menu} onClose={close} />

      {batchProgress && (
        <div className="batch-bar">
          <div className="progress-track">
            <div
              className="progress-fill"
              style={{ width: `${(batchProgress.current / Math.max(batchProgress.total, 1)) * 100}%` }}
            />
          </div>
          <div className="progress-text">
            {batchStatus === "running"
              ? `${batchProgress.current}/${batchProgress.total} ${batchProgress.file}`
              : batchStatus === "cancelled"
                ? t("detail.progressAborted", { c: batchProgress.current, t: batchProgress.total })
                : batchFailed > 0
                  ? t("detail.progressFailed", { c: batchProgress.current, t: batchProgress.total, f: batchFailed })
                  : t("detail.progressDone", { c: batchProgress.current, t: batchProgress.total })}
          </div>
        </div>
      )}
    </div>
  );
}
import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, startBatchTrace, presetFullParams, MODE_PRESETS } from "../api.js";
import { getCachedTrace, setCachedTrace } from "../traceCache.js";
import AppHeader from "../components/AppHeader.jsx";

/**
 * Cache marker for smart-batch results, per preset family. The winning params
 * differ per image, so smart results are keyed by this stable marker (for the
 * "already done?" pre-check) and additionally under the winning params (so
 * the editor's smart flow hits the cache).
 */
const smartCacheKey = (family) => ({ __smart: family });

/**
 * Crop an SVG to one of its quarters by rewriting the viewBox.
 * quarter: "tr" = top-right, "br" = bottom-right.
 * Falls back to the width/height attributes when no viewBox exists.
 */
function cropSvgQuarter(svg, quarter) {
  const vb = svg.match(/viewBox="([^"]+)"/);
  if (vb) {
    const [x, y, w, h] = vb[1].split(/[\s,]+/).map(Number);
    if ([x, y, w, h].some(n => !Number.isFinite(n))) return svg;
    const nx = x + w / 2;
    const ny = quarter === "tr" ? y : y + h / 2;
    return svg.replace(vb[0], `viewBox="${nx} ${ny} ${w / 2} ${h / 2}"`);
  }
  const wm = svg.match(/width="([\d.]+)(px)?"/);
  const hm = svg.match(/height="([\d.]+)(px)?"/);
  if (wm && hm) {
    const w = parseFloat(wm[1]), h = parseFloat(hm[1]);
    const ny = quarter === "tr" ? 0 : h / 2;
    return svg.replace(/<svg/, `<svg viewBox="${w / 2} ${ny} ${w / 2} ${h / 2}"`);
  }
  return svg;
}

/**
 * A single image card that lazy-loads its thumbnail only when scrolled near
 * the viewport. In Tauri mode each thumbnail is a full-resolution image read
 * over IPC, so eagerly loading a 100-image project would stall the UI and
 * balloon memory; IntersectionObserver keeps only the visible cards loaded.
 */
function ImageCard({ projectId, filename, preview, onClick }) {
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
    <div className="image-card" ref={cardRef} onClick={onClick}>
      {preview?.result ? (
        // Single-preset batch result (bw / gradient / lowPoly / smart), shown full on the right.
        <div className="img-wrap split">
          <div className="split-left">
            {url && <img src={url} alt={filename} />}
          </div>
          <div className="split-right">
            <div className="split-panel">
              <div className="svg-mini" dangerouslySetInnerHTML={{ __html: preview.result }} />
              {preview.label && <div className="mini-label">{preview.label}</div>}
            </div>
          </div>
        </div>
      ) : preview ? (
        <div className="img-wrap split">
          <div className="split-left">
            {url && <img src={url} alt={filename} />}
          </div>
          <div className="split-right">
            <div className="split-panel">
              {preview.line
                ? <div className="svg-mini" dangerouslySetInnerHTML={{ __html: cropSvgQuarter(preview.line, "tr") }} />
                : <div className="mini-empty">线稿</div>}
            </div>
            <div className="split-panel">
              {preview.color
                ? <div className="svg-mini" dangerouslySetInnerHTML={{ __html: cropSvgQuarter(preview.color, "br") }} />
                : <div className="mini-empty">色稿</div>}
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
  const { projectId } = useParams();
  const navigate = useNavigate();
  const [project, setProject] = useState(null);
  // Batch: the user picks a preset family (what output they want), then
  // chooses fixed params or smart tuning (best params within that family).
  const [batchPreset, setBatchPreset] = useState("lineArt");
  const [batchSmart, setBatchSmart] = useState(false);
  const [batchStatus, setBatchStatus] = useState(null);
  const [batchProgress, setBatchProgress] = useState(null);
  const [batchFailed, setBatchFailed] = useState(0);
  const [error, setError] = useState(null);
  // Cached line/color SVGs per image, for the split preview on processed cards
  const [previews, setPreviews] = useState({});
  const batchHandleRef = useRef(null);
  const cancelRef = useRef(false);

  useEffect(() => {
    // Load project info (image thumbnails are lazy-loaded per card below).
    (async () => {
      try {
        const { listProjects } = await import("../api.js");
        const data = await listProjects();
        const proj = (data.projects || []).find(p => p.id === projectId);
        if (proj) {
          setProject(proj);
        } else {
          // Tauri mode: project id is the folder path
          setProject({ id: projectId, name: projectId.split("/").pop(), path: projectId, images: [] });
          const { invoke } = await import("@tauri-apps/api/core");
          const images = await invoke("list_images", { folderPath: projectId });
          setProject(p => ({ ...p, images: images.map(i => i.name), imageCount: images.length }));
        }
      } catch (e) {
        setError(e.message);
      }
    })();
  }, [projectId]);

  // Load cached line/color results (fixed preset params or smart results)
  // for the split preview
  useEffect(() => {
    if (!project?.images?.length) return;
    let cancelled = false;
    (async () => {
      const found = {};
      for (const name of project.images) {
        const imageId = `${projectId}/${name}`;
        const [lineFixed, lineSmart, colorFixed, colorSmart] = await Promise.all([
          getCachedTrace(imageId, presetFullParams("lineArt")).catch(() => null),
          getCachedTrace(imageId, smartCacheKey("lineArt")).catch(() => null),
          getCachedTrace(imageId, presetFullParams("color")).catch(() => null),
          getCachedTrace(imageId, smartCacheKey("color")).catch(() => null),
        ]);
        const line = (lineFixed || lineSmart)?.svg || null;
        const color = (colorFixed || colorSmart)?.svg || null;
        if (line || color) found[name] = { line, color };
      }
      if (!cancelled) setPreviews(found);
    })();
    return () => { cancelled = true; };
  }, [project, projectId]);

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
    // Cache-first: only send images without a same-params result to the backend
    const plan = [];
    for (const job of jobs) {
      const missing = [];
      for (const name of project.images) {
        const key = job.smart ? smartCacheKey(job.family) : job.params;
        const cached = await getCachedTrace(`${projectId}/${name}`, key).catch(() => null);
        if (!cached) missing.push(name);
      }
      if (missing.length) plan.push({ ...job, missing });
    }
    const total = plan.reduce((s, j) => s + j.missing.length, 0);
    if (total === 0) {
      setBatchStatus("done");
      setBatchFailed(0);
      setBatchProgress({ current: project.images.length, total: project.images.length, file: "" });
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
          if (slot === "result") update.label = MODE_PRESETS[job.family]?.label;
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
  }, [project, projectId, batchPreset, batchSmart]);

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

  if (!project) {
    return <div className="placeholder">{error ? error : "加载中..."}</div>;
  }

  const allDone = batchStatus === "done" && batchFailed === 0;
  const presetLabel = MODE_PRESETS[batchPreset]?.label || "";
  const batchLabel =
    batchStatus === "running" ? "批量处理中..."
    : allDone ? "✓ 已完成"
    : batchStatus === "cancelled" ? "⚡ 继续处理"
    : batchStatus === "done" ? "⚡ 重新处理失败项"
    : batchSmart ? `✨ 智能批量 · ${presetLabel}`
    : `⚡ 批量处理 · ${presetLabel}`;

  return (
    <div className="project-detail-page">
      <AppHeader
        title={project.name}
        meta={`${project.images?.length || 0} 张图片`}
        onBack={() => navigate("/")}
      >
        <select
          className="batch-preset-select"
          value={batchPreset}
          onChange={e => { setBatchPreset(e.target.value); setBatchStatus(null); }}
          disabled={batchStatus === "running"}
          title="批量预设：选择要生成的输出类型"
        >
          {Object.entries(MODE_PRESETS).map(([key, p]) => (
            <option key={key} value={key}>{p.label}</option>
          ))}
        </select>
        <div className="seg-control" title="智能调参：在所选预设内为每张图试算最佳参数">
          <button className={!batchSmart ? "active" : ""} onClick={() => { setBatchSmart(false); setBatchStatus(null); }}>固定</button>
          <button className={batchSmart ? "active" : ""} onClick={() => { setBatchSmart(true); setBatchStatus(null); }}>✨ 智能</button>
        </div>
        {batchStatus === "running" && (
          <button className="danger" onClick={onCancelBatch}>⏹ 中止</button>
        )}
        <button className="primary" onClick={onBatchTrace} disabled={batchStatus === "running" || allDone}>
          {batchLabel}
        </button>
      </AppHeader>

      <div className="image-grid-scroll">
        <div className="image-grid">
          {(project.images || []).map(filename => (
            <ImageCard
              key={filename}
              projectId={projectId}
              filename={filename}
              preview={previews[filename]}
              onClick={() => navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(filename)}`)}
            />
          ))}
        </div>
      </div>

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
                ? `已中止: ${batchProgress.current}/${batchProgress.total}`
                : batchFailed > 0
                  ? `完成: ${batchProgress.current}/${batchProgress.total} · 失败 ${batchFailed}`
                  : `完成: ${batchProgress.current}/${batchProgress.total}`}
          </div>
        </div>
      )}
    </div>
  );
}
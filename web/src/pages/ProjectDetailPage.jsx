import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, startBatchTrace, presetFullParams } from "../api.js";
import { getCachedTrace, setCachedTrace } from "../traceCache.js";
import AppHeader from "../components/AppHeader.jsx";

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

export default function ProjectDetailPage() {
  const { projectId } = useParams();
  const navigate = useNavigate();
  const [project, setProject] = useState(null);
  const [imageUrls, setImageUrls] = useState({});
  const [batchStatus, setBatchStatus] = useState(null);
  const [batchProgress, setBatchProgress] = useState(null);
  const [batchFailed, setBatchFailed] = useState(0);
  const [error, setError] = useState(null);
  // Cached line/color SVGs per image, for the split preview on processed cards
  const [previews, setPreviews] = useState({});
  const batchHandleRef = useRef(null);
  const cancelRef = useRef(false);

  useEffect(() => {
    // Load project info and image URLs
    (async () => {
      try {
        const { listProjects } = await import("../api.js");
        const data = await listProjects();
        const proj = (data.projects || []).find(p => p.id === projectId);
        if (proj) {
          setProject(proj);
          // Load image URLs
          const urls = {};
          for (const img of proj.images) {
            try { urls[img] = await getImageUrl(projectId, img); } catch {}
          }
          setImageUrls(urls);
        } else {
          // Tauri mode: project id is the folder path
          setProject({ id: projectId, name: projectId.split("/").pop(), path: projectId, images: [] });
          const { invoke } = await import("@tauri-apps/api/core");
          const images = await invoke("list_images", { folderPath: projectId });
          setProject(p => ({ ...p, images: images.map(i => i.name), imageCount: images.length }));
          const urls = {};
          for (const img of images) {
            try { urls[img.name] = await getImageUrl(projectId, img.name); } catch {}
          }
          setImageUrls(urls);
        }
      } catch (e) {
        setError(e.message);
      }
    })();
  }, [projectId]);

  // Load cached line/color results (same params as batch trace) for the split preview
  useEffect(() => {
    if (!project?.images?.length) return;
    let cancelled = false;
    (async () => {
      const found = {};
      for (const name of project.images) {
        const imageId = `${projectId}/${name}`;
        const [line, color] = await Promise.all([
          getCachedTrace(imageId, presetFullParams("lineArt")).catch(() => null),
          getCachedTrace(imageId, presetFullParams("color")).catch(() => null),
        ]);
        if (line || color) found[name] = { line: line?.svg || null, color: color?.svg || null };
      }
      if (!cancelled) setPreviews(found);
    })();
    return () => { cancelled = true; };
  }, [project, projectId]);

  const onBatchTrace = useCallback(async () => {
    if (!project) return;
    // Two passes per image: line art, then color fill (both feed the split preview)
    const jobs = [
      { pkey: "line", params: presetFullParams("lineArt") },
      { pkey: "color", params: presetFullParams("color") },
    ];
    // Cache-first: only send images without a same-params result to the backend
    const plan = [];
    for (const job of jobs) {
      const missing = [];
      for (const name of project.images) {
        const cached = await getCachedTrace(`${projectId}/${name}`, job.params).catch(() => null);
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
      const handle = startBatchTrace(projectId, job.params, job.missing);
      batchHandleRef.current = handle;
      handle.setOnEvent((event) => {
        if (event.type === "processing") {
          setBatchProgress({ current: doneCount, total, file: event.file });
        } else if (event.type === "done") {
          doneCount += 1;
          setBatchProgress(p => ({ ...p, current: doneCount }));
          // Persist the generated result so the editor page hits the cache
          setCachedTrace(`${projectId}/${event.file}`, job.params, {
            svg: event.svg,
            width: event.width,
            height: event.height,
            shapeCount: event.shapeCount ?? event.shape_count,
            layerCount: (event.layers || []).length,
            layers: event.layers || [],
          }).catch(() => {});
          // Refresh the split preview for this card
          setPreviews(prev => ({
            ...prev,
            [event.file]: { ...(prev[event.file] || {}), [job.pkey]: event.svg },
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
  }, [project, projectId]);

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
  const batchLabel =
    batchStatus === "running" ? "批量处理中..."
    : allDone ? "✓ 已完成"
    : batchStatus === "cancelled" ? "⚡ 继续处理"
    : batchStatus === "done" ? "⚡ 重新处理失败项"
    : "⚡ 批量处理";

  return (
    <div className="project-detail-page">
      <AppHeader
        title={project.name}
        meta={`${project.images?.length || 0} 张图片`}
        onBack={() => navigate("/")}
      >
        {batchStatus === "running" && (
          <button className="danger" onClick={onCancelBatch}>⏹ 中止</button>
        )}
        <button className="primary" onClick={onBatchTrace} disabled={batchStatus === "running" || allDone}>
          {batchLabel}
        </button>
      </AppHeader>

      <div className="image-grid">
        {(project.images || []).map(filename => {
          const preview = previews[filename];
          return (
            <div
              key={filename}
              className="image-card"
              onClick={() => navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(filename)}`)}
            >
              {preview ? (
                <div className="img-wrap split">
                  <div className="split-left">
                    {imageUrls[filename] && <img src={imageUrls[filename]} alt={filename} />}
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
                  {imageUrls[filename] && <img src={imageUrls[filename]} alt={filename} />}
                </div>
              )}
              <div className="name">{filename}</div>
            </div>
          );
        })}
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
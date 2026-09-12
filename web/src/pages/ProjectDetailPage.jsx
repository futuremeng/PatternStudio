import React, { useEffect, useState, useCallback } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, startBatchTrace, MODE_PRESETS } from "../api.js";
import { getCachedTrace, setCachedTrace } from "../traceCache.js";
import AppHeader from "../components/AppHeader.jsx";

export default function ProjectDetailPage() {
  const { projectId } = useParams();
  const navigate = useNavigate();
  const [project, setProject] = useState(null);
  const [imageUrls, setImageUrls] = useState({});
  const [batchStatus, setBatchStatus] = useState(null);
  const [batchProgress, setBatchProgress] = useState(null);
  const [error, setError] = useState(null);

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

  const onBatchTrace = useCallback(async () => {
    if (!project) return;
    const params = MODE_PRESETS.color.params;
    // Cache-first: only send images without a same-params result to the backend
    const missing = [];
    for (const name of project.images) {
      const cached = await getCachedTrace(`${projectId}/${name}`, params).catch(() => null);
      if (!cached) missing.push(name);
    }
    if (missing.length === 0) {
      setBatchStatus("done");
      setBatchProgress({ current: project.images.length, total: project.images.length, file: "" });
      return;
    }
    setBatchStatus("running");
    setBatchProgress({ current: 0, total: missing.length, file: "" });

    const handle = startBatchTrace(projectId, params, missing);
    handle.setOnEvent((event) => {
      if (event.type === "start") {
        setBatchProgress({ current: 0, total: event.total, file: "" });
      } else if (event.type === "processing") {
        setBatchProgress({ current: event.index, total: event.total, file: event.file });
      } else if (event.type === "done") {
        setBatchProgress(p => ({ ...p, current: p.current + 1 }));
        // Persist the generated result so the editor page hits the cache
        setCachedTrace(`${projectId}/${event.file}`, params, {
          svg: event.svg,
          width: event.width,
          height: event.height,
          shapeCount: event.shapeCount ?? event.shape_count,
          layerCount: (event.layers || []).length,
          layers: event.layers || [],
        }).catch(() => {});
      } else if (event.type === "error") {
        setBatchProgress(p => ({ ...p, current: p.current + 1 }));
      } else if (event.type === "complete") {
        setBatchStatus("done");
        setBatchProgress({ current: event.total, total: event.total, file: "" });
      }
    });
    await handle.start;
  }, [project, projectId]);

  if (!project) {
    return <div className="placeholder">{error ? error : "加载中..."}</div>;
  }

  return (
    <div className="project-detail-page">
      <AppHeader
        title={project.name}
        meta={`${project.images?.length || 0} 张图片`}
        onBack={() => navigate("/")}
      >
        <button className="primary" onClick={onBatchTrace} disabled={batchStatus === "running"}>
          {batchStatus === "running" ? "批量处理中..." : "⚡ 批量处理"}
        </button>
      </AppHeader>

      <div className="image-grid">
        {(project.images || []).map(filename => (
          <div
            key={filename}
            className="image-card"
            onClick={() => navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(filename)}`)}
          >
            <div className="img-wrap">
              {imageUrls[filename] && <img src={imageUrls[filename]} alt={filename} />}
            </div>
            <div className="name">{filename}</div>
          </div>
        ))}
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
              : `完成: ${batchProgress.current}/${batchProgress.total}`}
          </div>
        </div>
      )}
    </div>
  );
}
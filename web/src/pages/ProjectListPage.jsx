import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { openProject, listProjects, deleteProject, getImageUrl } from "../api.js";
import AppHeader from "../components/AppHeader.jsx";
import { useI18n } from "../i18n/index.jsx";

export default function ProjectListPage() {
  const { t } = useI18n();
  const [projects, setProjects] = useState([]);
  const [thumbs, setThumbs] = useState({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const navigate = useNavigate();

  useEffect(() => {
    loadProjects();
  }, []);

  async function loadProjects() {
    try {
      const data = await listProjects();
      const projs = data.projects || [];
      setProjects(projs);
      // Load thumbnails (first image of each project)
      const newThumbs = {};
      for (const p of projs) {
        if (p.images && p.images.length > 0) {
          try {
            newThumbs[p.id] = await getImageUrl(p.id, p.images[0]);
          } catch {}
        }
      }
      setThumbs(newThumbs);
    } catch (e) {
      console.error("Failed to load projects:", e);
    }
  }

  async function onOpenProject() {
    setBusy(true);
    setError(null);
    try {
      const project = await openProject();
      navigate(`/project/${encodeURIComponent(project.id)}`);
    } catch (e) {
      if (e.message !== "cancelled") setError(e.message);
    } finally {
      setBusy(false);
    }
  }

  async function onRemoveProject(e, projectId) {
    e.stopPropagation();
    try {
      await deleteProject(projectId);
      setProjects(prev => prev.filter(p => p.id !== projectId));
    } catch (err) {
      console.error("Failed to remove project:", err);
    }
  }

  return (
    <div className="project-list-page">
      <AppHeader title={t("projects.title")}>
        <button className="open-project-btn" onClick={onOpenProject} disabled={busy}>
          {busy ? t("projects.opening") : t("projects.open")}
        </button>
      </AppHeader>

      <div className="project-list-body">
        {error && <div className="error">{error}</div>}
        {projects.length === 0 ? (
          <div className="empty-state">
            <div className="empty-icon">📁</div>
            <p>{t("projects.empty")}</p>
          </div>
        ) : (
          <div className="project-grid">
            {projects.map(p => (
              <div
                key={p.id}
                className="project-card"
                onClick={() => navigate(`/project/${encodeURIComponent(p.id)}`)}
              >
                <div className="project-thumb">
                  {thumbs[p.id] ? (
                    <img src={thumbs[p.id]} alt={p.name} />
                  ) : (
                    <div className="thumb-placeholder">🖼️</div>
                  )}
                  <button
                    className="remove-btn"
                    onClick={(e) => onRemoveProject(e, p.id)}
                    title={t("projects.remove")}
                  >
                    ×
                  </button>
                </div>
                <div className="project-info">
                  <h3>{p.name}</h3>
                  <div className="count">{t("imageCount", { n: p.imageCount })}</div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
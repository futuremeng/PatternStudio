import React, { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { openProject, listProjects, deleteProject, getImageUrl } from "../api.js";
import { errMsg } from "../editorUtils.js";
import AppHeader from "../components/AppHeader.jsx";
import ConfirmModal from "../components/ConfirmModal.jsx";
import ContextMenu, { useContextMenu } from "../components/ContextMenu.jsx";
import { IconFolder, IconImage } from "../components/icons.jsx";
import { useI18n } from "../i18n/index.jsx";

export default function ProjectListPage() {
  const { t } = useI18n();
  const { menu, open, close } = useContextMenu();
  const [projects, setProjects] = useState([]);
  const [thumbs, setThumbs] = useState({});
  // Distinguishes "still loading" from "no projects", so the empty state does
  // not flash at the start of every visit.
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  // The project whose removal is being confirmed (null = no prompt). Removing
  // one drops it from the list, so it is always confirmed first.
  const [removeTarget, setRemoveTarget] = useState(null);
  const navigate = useNavigate();

  useEffect(() => {
    loadProjects();
  }, []);

  async function loadProjects() {
    setLoading(true);
    setError(null);
    try {
      const data = await listProjects();
      const projs = data.projects || [];
      setProjects(projs);
      // Thumbnails (first image of each project) are fetched concurrently: a
      // serial loop made a list of N folders cost N round trips before the
      // first picture appeared. Each card fills in as its own image arrives.
      await Promise.all(projs.map(async p => {
        if (!p.images || p.images.length === 0) return;
        try {
          const u = await getImageUrl(p.id, p.images[0]);
          setThumbs(prev => ({ ...prev, [p.id]: u }));
        } catch { /* the placeholder stays visible */ }
      }));
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setLoading(false);
    }
  }

  async function onOpenProject() {
    setBusy(true);
    setError(null);
    try {
      const project = await openProject();
      navigate(`/project/${encodeURIComponent(project.id)}`);
    } catch (e) {
      if (e.message !== "cancelled") setError(errMsg(e));
    } finally {
      setBusy(false);
    }
  }

  function onRemoveProject() {
    const target = removeTarget;
    setRemoveTarget(null);
    if (!target) return;
    deleteProject(target.id).then(() => {
      setProjects(prev => prev.filter(p => p.id !== target.id));
    }).catch(e => setError(errMsg(e)));
  }

  return (
    <div className="project-list-page">
      <AppHeader title={t("projects.title")}>
        <button className="open-project-btn" onClick={onOpenProject} disabled={busy}>
          {busy ? t("projects.opening") : t("projects.open")}
        </button>
      </AppHeader>

      <div className="project-list-body">
        {error && projects.length > 0 && <div className="error">{error}</div>}
        {loading ? (
          <div className="project-grid">
            {[0, 1, 2, 3, 4, 5].map(i => (
              <div key={i} className="project-card skeleton" aria-hidden="true" />
            ))}
          </div>
        ) : error ? (
          // The first list fetch failed: do not claim "no projects" — the
          // error was a dead end, so offer a retry instead.
          <div className="error">
            {error}
            <button className="error-retry" onClick={loadProjects}>{t("retry")}</button>
          </div>
        ) : projects.length === 0 ? (
          <div className="empty-state">
            <div className="empty-icon"><IconFolder /></div>
            <p>{t("projects.empty")}</p>
          </div>
        ) : (
          <div className="project-grid">
            {projects.map(p => (
              <div
                key={p.id}
                className="project-card"
                onClick={() => navigate(`/project/${encodeURIComponent(p.id)}`)}
                onContextMenu={(e) =>
                  open(e, [
                    { label: t("ctx.open"), onSelect: () => navigate(`/project/${encodeURIComponent(p.id)}`) },
                    { label: t("projects.remove"), danger: true, onSelect: () => setRemoveTarget(p) },
                  ])
                }
              >
                <div className="project-thumb">
                  {thumbs[p.id] ? (
                    <img src={thumbs[p.id]} alt={p.name} />
                  ) : (
                    <div className="thumb-placeholder"><IconImage /></div>
                  )}
                  <button
                    className="remove-btn"
                    onClick={(e) => { e.stopPropagation(); setRemoveTarget(p); }}
                    title={t("projects.remove")}
                    aria-label={t("projects.remove")}
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

      {removeTarget && (
        <ConfirmModal
          title={t("projects.remove")}
          confirmLabel={t("projects.removeOk")}
          danger
          onConfirm={onRemoveProject}
          onCancel={() => setRemoveTarget(null)}
        >
          <p>{t("projects.removeConfirm", { name: removeTarget.name })}</p>
        </ConfirmModal>
      )}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  );
}

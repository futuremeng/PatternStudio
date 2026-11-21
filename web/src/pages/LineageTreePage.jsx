import React, { useEffect, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import AppHeader from "../components/AppHeader.jsx";
import LineageTreeView from "../components/LineageTree.jsx";
import { useProject } from "../hooks/useProject.js";
import { useI18n } from "../i18n/index.jsx";

/**
 * Per-image lineage tree page
 * (项目列表 → 项目详情(原图列表) → 某原图的衍生树 → 图片编辑).
 * Shows one original image (from the route) with its derived images nested
 * below; any image in the tree (original, confirmed derived, or
 * auto-generated) opens in the editor.
 */
export default function LineageTreePage() {
  const { t } = useI18n();
  const { projectId, filename } = useParams();
  const navigate = useNavigate();
  const [refreshKey, setRefreshKey] = useState(0);
  const { project, error } = useProject(projectId, refreshKey);
  // Tree filter: "confirmed" = only user-saved images, "all" = also the
  // auto-generated previews (badged by type). Persisted per browser.
  const [treeFilter, setTreeFilter] = useState(() => {
    try { return localStorage.getItem("ps.treeFilter") === "all" ? "all" : "confirmed"; } catch { return "confirmed"; }
  });
  useEffect(() => {
    try { localStorage.setItem("ps.treeFilter", treeFilter); } catch { /* ignore */ }
  }, [treeFilter]);

  if (!project) {
    // Same dead-end fix as the detail page: keep the header so Back works,
    // and retry re-reads the folder.
    return (
      <div className="project-detail-page">
        <AppHeader
          title={filename}
          onBack={() => navigate(`/project/${encodeURIComponent(projectId)}`)}
        />
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

  const openImage = (name) =>
    navigate(`/project/${encodeURIComponent(projectId)}/image/${encodeURIComponent(name)}`);

  return (
    <div className="project-detail-page">
      <AppHeader
        title={filename}
        meta={project.name}
        onBack={() => navigate(`/project/${encodeURIComponent(projectId)}`)}
      >
        <div className="seg-control" title={t("detail.treeFilterTitle")}>
          <button className={treeFilter === "confirmed" ? "active" : ""} onClick={() => setTreeFilter("confirmed")}>{t("detail.treeConfirmed")}</button>
          <button className={treeFilter === "all" ? "active" : ""} onClick={() => setTreeFilter("all")}>{t("detail.treeAll")}</button>
        </div>
      </AppHeader>

      <div className="image-list-scroll">
        <LineageTreeView project={project} filter={treeFilter} root={filename} onOpen={openImage} />
      </div>
    </div>
  );
}
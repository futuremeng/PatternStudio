import { useEffect, useState } from "react";
import { listProjects } from "../api.js";

/**
 * Load one project (by id) for the detail / lineage-tree pages.
 * Web mode: looked up in the server's project list. Tauri mode: the id is
 * the folder path itself; images + lineage come from dedicated commands
 * (also covers folders that are not in the recent-projects list).
 */
export function useProject(projectId) {
  const [project, setProject] = useState(null);
  const [error, setError] = useState(null);

  useEffect(() => {
    let cancelled = false;
    setProject(null);
    setError(null);
    (async () => {
      try {
        const data = await listProjects();
        const proj = (data.projects || []).find(p => p.id === projectId);
        if (cancelled) return;
        if (proj) {
          setProject(proj);
        } else {
          // Tauri mode: project id is the folder path
          setProject({ id: projectId, name: projectId.split("/").pop(), path: projectId, images: [], lineage: [] });
          const { invoke } = await import("@tauri-apps/api/core");
          const images = await invoke("list_images", { folderPath: projectId });
          let lineage = [];
          try {
            lineage = await invoke("list_lineage", { folderPath: projectId });
          } catch {
            /* lineage is optional; the tree degrades to flat rows */
          }
          if (cancelled) return;
          setProject(p => ({ ...p, images: images.map(i => i.name), lineage, imageCount: images.length }));
        }
      } catch (e) {
        if (!cancelled) setError(e.message);
      }
    })();
    return () => { cancelled = true; };
  }, [projectId]);

  return { project, error };
}
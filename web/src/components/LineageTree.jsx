import React, { useEffect, useState, useRef } from "react";
import { getImageUrl } from "../api.js";
import { buildLineageTree, countTreeNodes } from "../lineageTree.js";
import { useI18n } from "../i18n/index.jsx";

/**
 * Lazy thumbnail for the lineage tree: fetches the image only when scrolled
 * near the viewport (same IntersectionObserver pattern as the grid's
 * ImageCard). `size`: "row" (original) or "node" (derived image).
 */
export function LazyThumb({ projectId, filename, size = "node", missing = false }) {
  const [url, setUrl] = useState(null);
  const ref = useRef(null);

  useEffect(() => {
    if (url || missing) return;
    const el = ref.current;
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
  }, [projectId, filename, url, missing]);

  useEffect(() => {
    return () => {
      if (url) URL.revokeObjectURL(url);
    };
  }, [url]);

  return (
    <div className={`lineage-thumb ${size}`} ref={ref}>
      {url && <img src={url} alt={filename} />}
      {missing && <span className="lineage-thumb-missing">?</span>}
    </div>
  );
}

function lineageTooltip(node, t) {
  const parts = [node.name];
  if (node.createdAt) parts.push(node.createdAt.replace("T", " ").replace(/\.\d+Z$/, "Z"));
  const meta = node.meta || {};
  if (meta.prompt) parts.push(`prompt: ${meta.prompt}`);
  if (meta.seed != null) parts.push(`seed: ${meta.seed}`);
  return parts.join("\n");
}

function kindLabel(kind, t) {
  if (kind === "enhance") return t("lineage.enhance");
  if (kind === "redraw") return t("lineage.redraw");
  return kind;
}

/** One row of the tree: an original image plus its derived lineage tree. */
function LineageRow({ row, projectId, onOpen }) {
  const { t } = useI18n();
  const [expanded, setExpanded] = useState(true);
  const count = countTreeNodes(row.nodes);
  const openable = !row.missing;
  return (
    <div className="lineage-row">
      <div
        className="lineage-row-head"
        onClick={() => openable && onOpen(row.original)}
        role={openable ? "button" : undefined}
      >
        <button
          type="button"
          className="lineage-toggle"
          aria-label={expanded ? "collapse" : "expand"}
          onClick={(e) => { e.stopPropagation(); setExpanded(v => !v); }}
        >
          {expanded ? "▾" : "▸"}
        </button>
        <LazyThumb projectId={projectId} filename={row.original} size="row" missing={row.missing} />
        <span className="lineage-name" title={row.original}>
          {row.missing ? `${t("detail.missingOriginal")} (${row.original})` : row.original}
        </span>
        {count > 0 && <span className="lineage-count">{t("detail.derivedCount", { n: count })}</span>}
      </div>
      {expanded && row.nodes.length > 0 && (
        <div className="lineage-tree">
          {row.nodes.map(n => (
            <LineageNode key={n.name} node={n} projectId={projectId} onOpen={onOpen} />
          ))}
        </div>
      )}
    </div>
  );
}

/** One node of the derived tree (recursive). */
function LineageNode({ node, projectId, onOpen }) {
  const { t } = useI18n();
  return (
    <div className="lineage-node-wrap">
      <div
        className={`lineage-node${node.openName ? " clickable" : ""}`}
        title={lineageTooltip(node, t)}
        onClick={() => node.openName && onOpen(node.openName)}
        role={node.openName ? "button" : undefined}
      >
        <LazyThumb projectId={projectId} filename={node.name} size="node" />
        <span className="lineage-name" title={node.name}>{node.name}</span>
        <span className={`lineage-badge kind-${node.kind}`}>{kindLabel(node.kind, t)}</span>
        {!node.confirmed && <span className="lineage-badge auto">{t("lineage.auto")}</span>}
      </div>
      {node.children.length > 0 && (
        <div className="lineage-children">
          {node.children.map(c => (
            <LineageNode key={c.name} node={c} projectId={projectId} onOpen={onOpen} />
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The lineage tree for a project: one row per original image with its
 * derived images nested below. `filter`: "confirmed" (only user-saved
 * images) or "all" (also the auto-generated previews, badged by type).
 * `root`: when set, only the row of that original image is shown (the
 * per-image tree page). `onOpen(filename)` is called for any image
 * (original, confirmed derived, or auto-generated) so the caller can open
 * it in the editor.
 */
export default function LineageTreeView({ project, filter, root = null, onOpen }) {
  const rows = buildLineageTree(project.images, project.lineage, filter)
    .filter(row => !root || row.original === root);
  return (
    <div className="image-list">
      {rows.map(row => (
        <LineageRow
          key={row.original}
          row={row}
          projectId={project.id}
          onOpen={onOpen}
        />
      ))}
    </div>
  );
}
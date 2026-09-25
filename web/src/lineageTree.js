/**
 * Lineage tree building for the project list view (pure functions, unit
 * tested in lineageTree.test.js).
 *
 * Entry shape (from the backend): { file, root, parent, kind, confirmed,
 * confirmedFile, createdAt, meta } — `file` is the generated filename (or the
 * top-level filename for legacy inferred entries).
 */

/**
 * Build the lineage tree for the list view.
 * `images`: top-level filenames (originals + confirmed derived images).
 * `lineage`: entries as above.
 * `filter`: "confirmed" (only user-saved images) or "all" (also the
 * auto-generated previews).
 * Returns rows: [{ original, missing, nodes }] — one row per original image,
 * plus one row per root whose original file no longer exists.
 */
export function buildLineageTree(images, lineage, filter) {
  const visible = (lineage || []).filter(e => (filter === "all" ? true : !!e.confirmedFile));
  const nodes = new Map(); // display name -> node
  for (const e of visible) {
    const name = e.confirmedFile || e.file;
    nodes.set(name, {
      name,
      openName: e.confirmedFile || e.file, // any image (confirmed or auto-generated) opens in the editor
      kind: e.kind,
      confirmed: !!e.confirmedFile,
      meta: e.meta,
      createdAt: e.createdAt || "",
      parent: e.parent,
      root: e.root,
      children: [],
    });
  }
  const tops = [];
  for (const node of nodes.values()) {
    const parent = nodes.get(node.parent);
    if (parent && parent !== node) parent.children.push(node);
    else tops.push(node); // parent not visible (or self) → hang off the root
  }
  const byCreated = (a, b) =>
    (a.createdAt || "").localeCompare(b.createdAt || "") || a.name.localeCompare(b.name);
  tops.sort(byCreated);
  for (const n of nodes.values()) n.children.sort(byCreated);

  const confirmedFiles = new Set((lineage || []).map(e => e.confirmedFile).filter(Boolean));
  const imageSet = new Set(images || []);
  const rows = [];
  for (const orig of [...(images || [])].filter(n => !confirmedFiles.has(n)).sort()) {
    rows.push({ original: orig, missing: false, nodes: tops.filter(n => n.root === orig) });
  }
  const missingRoots = [...new Set(tops.map(n => n.root))].filter(r => !imageSet.has(r)).sort();
  for (const r of missingRoots) {
    rows.push({ original: r, missing: true, nodes: tops.filter(n => n.root === r) });
  }
  return rows;
}

/** Total number of nodes in a tree (all depths). */
export function countTreeNodes(nodes) {
  let n = 0;
  const walk = (list) => { for (const x of list) { n += 1; walk(x.children); } };
  walk(nodes);
  return n;
}
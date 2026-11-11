import { useEffect, useState, useCallback, useRef } from "react";
import { serializeEditedSvg, dominantFill } from "../editorUtils.js";

/**
 * Patch selection & dragging on the vector preview: marquee / ⌘-click
 * selection, per-path translate transforms, control-point handles, and
 * snapshotting the edited canvas as a new layer.
 *
 * All interaction state lives in refs (no per-frame re-renders); the hook
 * drives the SVG rendered inside `svgContainerRef` (CompareViewer forwards
 * the ref) imperatively. `onSnapshotLayer` receives
 * `{ shapeCount, dominantColor, previewSvg }` for a new edited layer.
 *
 * Patch edits are preview-local until they are applied, so they are also
 * undoable: every move / delete pushes an undo record holding the transform
 * map (and the removed nodes) as they were *before* the edit. `applyEdits`
 * hands the serialized canvas to the caller and marks the edits applied,
 * `undoEdit` steps back one record, `discardEdits` steps back to the pristine
 * canvas the editing object holds.
 */
export function usePatchEditing({ result, svgKey, compareMode, canvasSize, onSnapshotLayer }) {
  // The SVG actually rendered in the preview: the layered result, or one
  // exported layer when the user picked it as the editing object. Everything
  // here keys off that string, so switching the editing object also drops the
  // selection and the per-path transforms of the previous canvas.
  const canvasSvg = svgKey ?? result?.svg;
  const [tool, setTool] = useState("none");
  const svgContainerRef = useRef(null);
  const pathTransformsRef = useRef(new Map()); // pathIdx → { dx, dy } in SVG coords
  const dragRef = useRef(null); // { idx, startClientX, startClientY, origDx, origDy }
  const selectedPathIdxsRef = useRef(new Set()); // Set of selected path indices
  // Undo records for the preview-local edits: each holds the transform map (and
  // the nodes removed by that one edit) as they were *before* it happened.
  const undoRef = useRef([]);
  const [undoDepth, setUndoDepth] = useState(0);
  // The transform attribute a path had before the first drag overwrote it
  // (usually none). Undo and discard restore exactly that, so a source SVG that
  // carries its own transforms is never flattened by an undo step.
  const origTransformsRef = useRef(new Map());

  const setUndoRecords = useCallback((records) => {
    undoRef.current = records;
    setUndoDepth(records.length);
  }, []);

  /** Record the state before an edit so it can be stepped back. */
  const pushUndo = useCallback((transforms, removals = []) => {
    const records = [...undoRef.current, { transforms, removals }];
    // Bound the history: a long drag session should not grow without end.
    setUndoRecords(records.length > 60 ? records.slice(-60) : records);
  }, [setUndoRecords]);

  const getSvgAndPaths = useCallback(() => {
    const container = svgContainerRef.current;
    if (!container) return null;
    const svg = container.querySelector("svg");
    if (!svg) return null;
    return { svg, paths: Array.from(svg.querySelectorAll("path")) };
  }, []);

  // Snapshot the current edited preview as a new, exportable layer.
  const saveAsLayer = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    onSnapshotLayer({
      shapeCount: res.paths.length,
      dominantColor: dominantFill(res.paths),
      // Pin the snapshot to the original canvas: the layer holds a subset of
      // the shapes, not a crop of the artwork.
      previewSvg: serializeEditedSvg(res.svg, canvasSize),
    });
  }, [getSvgAndPaths, canvasSize, onSnapshotLayer]);

  const clearPatchSelection = useCallback(() => {
    selectedPathIdxsRef.current = new Set();
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    paths.forEach(p => {
      p.style.stroke = "";
      p.style.strokeWidth = "";
      p.style.cursor = "";
    });
    svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
  }, [getSvgAndPaths]);

  // Highlight every path in the current selection set.
  const highlightPaths = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    const sel = selectedPathIdxsRef.current;
    res.paths.forEach((p, i) => {
      if (sel.has(i)) {
        p.style.stroke = "#41b6e6";
        p.style.strokeWidth = "2";
        p.style.cursor = "move";
      } else {
        p.style.stroke = "";
        p.style.strokeWidth = "";
        p.style.cursor = "pointer";
      }
    });
  }, [getSvgAndPaths]);

  // Render the 8 control points for each selected path.
  const renderControlPoints = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    svg.querySelectorAll(".patch-cp").forEach(el => el.remove());
    for (const idx of selectedPathIdxsRef.current) {
      const path = paths[idx];
      if (!path) continue;
      const t = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
      const bb = path.getBBox();
      const r = Math.max(3, Math.min(bb.width, bb.height) * 0.015);
      const mx = bb.x + bb.width / 2 + t.dx;
      const my = bb.y + bb.height / 2 + t.dy;
      const pts = [
        [bb.x + t.dx, bb.y + t.dy], [mx, bb.y + t.dy],
        [bb.x + bb.width + t.dx, bb.y + t.dy], [bb.x + bb.width + t.dx, my],
        [bb.x + bb.width + t.dx, bb.y + bb.height + t.dy], [mx, bb.y + bb.height + t.dy],
        [bb.x + t.dx, bb.y + bb.height + t.dy], [bb.x + t.dx, my],
      ];
      for (const [px, py] of pts) {
        const c = document.createElementNS("http://www.w3.org/2000/svg", "circle");
        c.setAttribute("cx", px); c.setAttribute("cy", py); c.setAttribute("r", r);
        c.setAttribute("fill", "#41b6e6"); c.setAttribute("stroke", "#fff");
        c.setAttribute("stroke-width", (r * 0.35).toFixed(1));
        c.classList.add("patch-cp"); c.style.pointerEvents = "none";
        svg.appendChild(c);
      }
    }
  }, [getSvgAndPaths]);

  /** Write every recorded transform onto the current path list. */
  const applyTransformsToDom = useCallback((paths, transforms) => {
    paths.forEach((p, i) => {
      const t = transforms.get(i);
      if (t && (t.dx || t.dy)) {
        p.setAttribute("transform", `translate(${t.dx}, ${t.dy})`);
      } else if (origTransformsRef.current.has(i)) {
        const orig = origTransformsRef.current.get(i);
        if (orig) p.setAttribute("transform", orig);
        else p.removeAttribute("transform");
      }
    });
  }, []);

  /** Remember what a path's transform looked like before we touch it. */
  const recordOrigTransform = useCallback((path, idx) => {
    if (!origTransformsRef.current.has(idx)) {
      origTransformsRef.current.set(idx, path.getAttribute("transform"));
    }
  }, []);

  /**
   * The edited canvas as a standalone SVG, ready for the caller to write into
   * the editing object. Selection highlights, control points and the marquee
   * are stripped; the snapshot stays on the original canvas.
   */
  const applyEdits = useCallback(() => {
    const res = getSvgAndPaths();
    if (!res) return null;
    return serializeEditedSvg(res.svg, canvasSize);
  }, [getSvgAndPaths, canvasSize]);

  /** Mark the pending edits as applied: the canvas is the new baseline. */
  const markEditsApplied = useCallback(() => {
    setUndoRecords([]);
    // The applied SVG now carries the offsets in its own transform attributes,
    // so the extra-translate map starts over at zero — otherwise the next drag
    // would add the applied offset a second time.
    pathTransformsRef.current.clear();
    origTransformsRef.current.clear();
    clearPatchSelection();
  }, [clearPatchSelection, setUndoRecords]);

  /** Step back one edit: re-insert what it removed, restore its transforms. */
  const undoEdit = useCallback(() => {
    const records = undoRef.current;
    if (!records.length) return;
    const op = records[records.length - 1];
    setUndoRecords(records.slice(0, -1));
    const container = svgContainerRef.current;
    if (!container) return;
    // Put the removed shapes back where they were, then re-read the path list:
    // the indices the record's transform map uses only line up again once the
    // removed shapes sit in their old places.
    for (const r of op.removals) {
      if (r.parent && container.contains(r.parent)) r.parent.insertBefore(r.node, r.next || null);
    }
    const res = getSvgAndPaths();
    if (!res) return;
    pathTransformsRef.current = op.transforms;
    applyTransformsToDom(res.paths, op.transforms);
    selectedPathIdxsRef.current = new Set(
      [...selectedPathIdxsRef.current].filter(i => i < res.paths.length)
    );
    highlightPaths();
    renderControlPoints();
  }, [getSvgAndPaths, highlightPaths, renderControlPoints, applyTransformsToDom, setUndoRecords]);

  /** Drop every pending edit: back to the canvas the editing object holds. */
  const discardEdits = useCallback(() => {
    while (undoRef.current.length) {
      const op = undoRef.current[undoRef.current.length - 1];
      const container = svgContainerRef.current;
      if (container) {
        for (const r of op.removals) {
          if (r.parent && container.contains(r.parent)) r.parent.insertBefore(r.node, r.next || null);
        }
        applyTransformsToDom(Array.from(container.querySelectorAll("path")), op.transforms);
      }
      undoRef.current = undoRef.current.slice(0, -1);
      pathTransformsRef.current = op.transforms;
    }
    setUndoRecords([]);
    // The map now holds the baseline again: the offsets of edits that were
    // applied earlier stay in it, so dragging those shapes keeps their place.
    clearPatchSelection();
  }, [clearPatchSelection, applyTransformsToDom, setUndoRecords]);

  // Reset patch selection when the SVG changes (new trace result or a new
  // editing object). Any pending edit belongs to the canvas that is gone.
  useEffect(() => {
    selectedPathIdxsRef.current = new Set();
    pathTransformsRef.current.clear();
    origTransformsRef.current.clear();
    setUndoRecords([]);
  }, [canvasSvg, setUndoRecords]);

  // Attach pointer interactions to SVG paths for patch selection & dragging.
  useEffect(() => {
    if (tool !== "select" || !canvasSvg) return;
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    if (!paths.length) return;

    // Re-apply existing transforms & highlight (e.g. after compare-mode switch)
    pathTransformsRef.current.forEach((t, idx) => {
      if (paths[idx]) {
        recordOrigTransform(paths[idx], idx);
        paths[idx].setAttribute("transform", `translate(${t.dx}, ${t.dy})`);
      }
    });
    if (selectedPathIdxsRef.current.size > 0) {
      highlightPaths();
      renderControlPoints();
    }

    // Convert a client point to SVG user-space coordinates. Uses the
    // element's actual screen CTM, which stays correct for any viewBox
    // (including the offset viewBox produced by "shrink canvas") and any
    // letterboxing — a manual rect/viewBox ratio would drift as soon as the
    // viewBox aspect ratio differs from the rendered element's.
    const clientToSvg = (clientX, clientY) => {
      const ctm = svg.getScreenCTM();
      if (!ctm) return null;
      const p = new DOMPoint(clientX, clientY).matrixTransform(ctm.inverse());
      return { x: p.x, y: p.y };
    };

    // Select every path whose (transformed) bounding box intersects the
    // marquee rect. With `additive` the result is unioned with the current
    // selection (⌘/Ctrl-drag); otherwise it replaces it.
    const applyMarqueeSelection = (rect, additive) => {
      const sel = additive ? new Set(selectedPathIdxsRef.current) : new Set();
      paths.forEach((p, i) => {
        const t = pathTransformsRef.current.get(i) || { dx: 0, dy: 0 };
        let bb;
        try { bb = p.getBBox(); } catch { return; } // skip detached/hidden paths
        const x0 = bb.x + t.dx, y0 = bb.y + t.dy;
        const x1 = bb.x + bb.width + t.dx, y1 = bb.y + bb.height + t.dy;
        if (x1 < rect.x || x0 > rect.x + rect.w || y1 < rect.y || y0 > rect.y + rect.h) return;
        sel.add(i);
      });
      selectedPathIdxsRef.current = sel;
      highlightPaths();
      renderControlPoints();
    };

    const onPathPointerDown = (e) => {
      if (e.button !== 0) return;
      const idx = paths.indexOf(e.target);
      if (idx === -1) return;
      e.preventDefault();
      e.stopPropagation();
      // ⌘/Ctrl+click: toggle this path in/out of the selection, no dragging.
      if (e.metaKey || e.ctrlKey) {
        const sel = new Set(selectedPathIdxsRef.current);
        if (sel.has(idx)) sel.delete(idx); else sel.add(idx);
        selectedPathIdxsRef.current = sel;
        highlightPaths();
        renderControlPoints();
        return;
      }
      selectedPathIdxsRef.current = new Set([idx]);
      highlightPaths();
      renderControlPoints();
      const t = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
      dragRef.current = { idx, sx: e.clientX, sy: e.clientY, ox: t.dx, oy: t.dy };
      // The state to go back to if the user undoes this drag.
      const before = new Map(pathTransformsRef.current);

      const onMove = (me) => {
        const d = dragRef.current;
        if (!d) return;
        const ctm = svg.getScreenCTM();
        if (!ctm) return;
        const inv = ctm.inverse();
        const start = new DOMPoint(d.sx, d.sy).matrixTransform(inv);
        const cur = new DOMPoint(me.clientX, me.clientY).matrixTransform(inv);
        const dx = d.ox + (cur.x - start.x);
        const dy = d.oy + (cur.y - start.y);
        pathTransformsRef.current.set(d.idx, { dx, dy });
        recordOrigTransform(paths[d.idx], d.idx);
        paths[d.idx].setAttribute("transform", `translate(${dx}, ${dy})`);
        renderControlPoints();
      };
      const onUp = () => {
        dragRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        // Only a drag that actually moved the shape is an edit worth undoing.
        const was = before.get(idx) || { dx: 0, dy: 0 };
        const now = pathTransformsRef.current.get(idx) || { dx: 0, dy: 0 };
        if (now.dx !== was.dx || now.dy !== was.dy) pushUndo(before);
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    // Marquee (rubber-band) selection: press on empty svg area and drag.
    // Paths stop propagation on pointerdown, so this only fires for the
    // background. A plain click on the background clears the selection.
    const onSvgPointerDown = (e) => {
      if (e.button !== 0) return;
      e.stopPropagation(); // don't let the viewport start panning
      const start = clientToSvg(e.clientX, e.clientY);
      if (!start) return;
      const additive = e.metaKey || e.ctrlKey;
      let marquee = null;
      let active = false;
      let last = start;

      const onMove = (me) => {
        const cur = clientToSvg(me.clientX, me.clientY);
        if (!cur) return;
        last = cur;
        if (!active) {
          if (Math.hypot(me.clientX - e.clientX, me.clientY - e.clientY) < 3) return;
          active = true;
          marquee = document.createElementNS("http://www.w3.org/2000/svg", "rect");
          marquee.setAttribute("class", "patch-marquee");
          marquee.setAttribute("fill", "rgba(65, 182, 230, 0.15)");
          marquee.setAttribute("stroke", "#41b6e6");
          marquee.setAttribute("stroke-width", "1");
          marquee.setAttribute("stroke-dasharray", "4 3");
          marquee.style.pointerEvents = "none";
          svg.appendChild(marquee);
        }
        const x = Math.min(start.x, cur.x);
        const y = Math.min(start.y, cur.y);
        const w = Math.abs(cur.x - start.x);
        const h = Math.abs(cur.y - start.y);
        marquee.setAttribute("x", x);
        marquee.setAttribute("y", y);
        marquee.setAttribute("width", w);
        marquee.setAttribute("height", h);
      };
      const onUp = () => {
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
        if (marquee) marquee.remove();
        if (active) {
          const x = Math.min(start.x, last.x);
          const y = Math.min(start.y, last.y);
          applyMarqueeSelection(
            { x, y, w: Math.abs(last.x - start.x), h: Math.abs(last.y - start.y) },
            additive
          );
        } else if (!additive) {
          clearPatchSelection();
        }
      };
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
    };

    const onKeyDown = (e) => {
      const tag = e.target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      // ⌘/Ctrl+Z steps back one preview edit. Guarded by the canvas being live:
      // in advanced mode the advanced editor owns its own undo stack.
      if ((e.metaKey || e.ctrlKey) && (e.key === "z" || e.key === "Z")) {
        if (!svgContainerRef.current?.isConnected) return;
        e.preventDefault();
        undoEdit();
        return;
      }
      if (e.key === "Escape") { clearPatchSelection(); return; }
      if ((e.key === "Delete" || e.key === "Backspace") && selectedPathIdxsRef.current.size > 0) {
        if (!svgContainerRef.current?.isConnected) return;
        e.preventDefault();
        const current = getSvgAndPaths();
        if (!current) return;
        // Remember what is about to go, so ⌘Z can put it back.
        const before = new Map(pathTransformsRef.current);
        const removals = [];
        // Remove in descending order so earlier indices stay valid.
        const removed = [...selectedPathIdxsRef.current].sort((a, b) => b - a);
        for (const idx of removed) {
          const node = current.paths[idx];
          if (node) removals.push({ node, parent: node.parentNode, next: node.nextSibling });
          if (node) node.remove();
          pathTransformsRef.current.delete(idx);
        }
        // Removing paths shifts the remaining indices: remap transforms and
        // the selection set accordingly.
        const remap = (i) => i - removed.filter(r => r < i).length;
        const newTransforms = new Map();
        pathTransformsRef.current.forEach((t, i) => newTransforms.set(remap(i), t));
        pathTransformsRef.current = newTransforms;
        selectedPathIdxsRef.current = new Set([...selectedPathIdxsRef.current].map(remap));
        highlightPaths();
        renderControlPoints();
        pushUndo(before, removals);
      }
    };

    paths.forEach(p => {
      p.style.pointerEvents = "auto";
      p.style.cursor = "pointer";
      p.style.touchAction = "none";
      p.addEventListener("pointerdown", onPathPointerDown);
    });
    svg.addEventListener("pointerdown", onSvgPointerDown);
    window.addEventListener("keydown", onKeyDown);

    return () => {
      paths.forEach(p => {
        p.style.pointerEvents = "";
        p.style.cursor = "";
        p.style.touchAction = "";
        p.removeEventListener("pointerdown", onPathPointerDown);
      });
      svg.removeEventListener("pointerdown", onSvgPointerDown);
      window.removeEventListener("keydown", onKeyDown);
      dragRef.current = null;
    };
  }, [tool, canvasSvg, compareMode, getSvgAndPaths, highlightPaths, renderControlPoints, clearPatchSelection, pushUndo, undoEdit, recordOrigTransform]);

  return {
    tool,
    setTool,
    svgContainerRef,
    saveAsLayer,
    // Preview-local edits: pending (undoDepth > 0) until applied by the caller.
    dirty: undoDepth > 0,
    canUndo: undoDepth > 0,
    applyEdits,
    markEditsApplied,
    undoEdit,
    discardEdits,
  };
}

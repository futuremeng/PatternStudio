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
 */
export function usePatchEditing({ result, compareMode, canvasSize, onSnapshotLayer }) {
  const [tool, setTool] = useState("none");
  const svgContainerRef = useRef(null);
  const pathTransformsRef = useRef(new Map()); // pathIdx → { dx, dy } in SVG coords
  const dragRef = useRef(null); // { idx, startClientX, startClientY, origDx, origDy }
  const selectedPathIdxsRef = useRef(new Set()); // Set of selected path indices

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

  // Reset patch selection when the SVG changes (new trace result).
  useEffect(() => {
    selectedPathIdxsRef.current = new Set();
    pathTransformsRef.current.clear();
  }, [result?.svg]);

  // Attach pointer interactions to SVG paths for patch selection & dragging.
  useEffect(() => {
    if (tool !== "select" || !result?.svg) return;
    const res = getSvgAndPaths();
    if (!res) return;
    const { svg, paths } = res;
    if (!paths.length) return;

    // Re-apply existing transforms & highlight (e.g. after compare-mode switch)
    pathTransformsRef.current.forEach((t, idx) => {
      if (paths[idx]) paths[idx].setAttribute("transform", `translate(${t.dx}, ${t.dy})`);
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
        paths[d.idx].setAttribute("transform", `translate(${dx}, ${dy})`);
        renderControlPoints();
      };
      const onUp = () => {
        dragRef.current = null;
        window.removeEventListener("pointermove", onMove);
        window.removeEventListener("pointerup", onUp);
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
      if (e.key === "Escape") { clearPatchSelection(); return; }
      if ((e.key === "Delete" || e.key === "Backspace") && selectedPathIdxsRef.current.size > 0) {
        e.preventDefault();
        const current = getSvgAndPaths();
        if (!current) return;
        // Remove in descending order so earlier indices stay valid.
        const removed = [...selectedPathIdxsRef.current].sort((a, b) => b - a);
        for (const idx of removed) {
          if (current.paths[idx]) current.paths[idx].remove();
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
  }, [tool, result?.svg, compareMode, getSvgAndPaths, highlightPaths, renderControlPoints, clearPatchSelection]);

  return { tool, setTool, svgContainerRef, saveAsLayer };
}

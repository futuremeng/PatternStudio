import React, {
  forwardRef,
  useEffect,
  useRef,
  useState,
  useCallback,
  useImperativeHandle,
} from "react";
import SvgCanvas from "@svgedit/svgcanvas";
import AdvancedToolbar from "./AdvancedToolbar.jsx";
import { useI18n } from "../i18n/index.jsx";

// Padding (px) kept around the fitted canvas inside the editor container.
const PAD = 16;

/**
 * Unwrap the top-level layer containers, hoisting every shape to be a
 * direct child of the <svg> root. Two kinds are recognized:
 *   - <g data-role="...">  – layer containers emitted by the patternstudio
 *     SVG writer
 *   - <g class="layer">    – layer containers created by SvgCanvas itself
 *     when it imports content (identifyLayers in core/draw.js)
 *
 * SvgCanvas only allows selecting elements that are direct children of the
 * current layer (getMouseTarget in @svgedit/svgcanvas/core/selection.js
 * walks up from the clicked element until its parent is the current layer),
 * so without this flattening the entire artwork would be selected as one
 * single object and individual lines could not be edited.
 *
 * Groups the user created via ⌘G carry neither marker and are left intact.
 */
function flattenLayerGroups(svgString) {
  try {
    const doc = new DOMParser().parseFromString(svgString, "image/svg+xml");
    const svg = doc.documentElement;
    if (!svg || svg.tagName !== "svg" || doc.querySelector("parsererror")) {
      return svgString;
    }
    let changed = false;
    for (const g of [...svg.children]) {
      const isLayerContainer =
        g.tagName === "g" &&
        (g.hasAttribute("data-role") || g.classList.contains("layer"));
      if (!isLayerContainer) continue;
      while (g.firstChild) svg.insertBefore(g.firstChild, g);
      svg.removeChild(g);
      changed = true;
    }
    return changed ? new XMLSerializer().serializeToString(svg) : svgString;
  } catch {
    return svgString;
  }
}

/**
 * Rebuild a clean, writer-style standalone SVG from the raw string returned
 * by SvgCanvas.getSvgString().
 *
 * The canvas export cannot be used as-is:
 *   - its root has no viewBox (width/height reflect the on-screen zoom, not
 *     the artwork size), so the static preview would render it at 1:1 scale
 *   - the artwork is wrapped in a <g class="layer"> container, so
 *     re-entering the advanced editor would treat it as one unselectable
 *     object again
 *
 * Returns null when the raw string cannot be parsed.
 */
function normalizeEditedSvg(rawSvg, w, h, x = 0, y = 0) {
  try {
    const doc = new DOMParser().parseFromString(rawSvg, "image/svg+xml");
    const root = doc.documentElement;
    if (!root || root.tagName !== "svg" || doc.querySelector("parsererror")) {
      return null;
    }
    const serializer = new XMLSerializer();
    const parts = [];
    const push = (el) => {
      // XMLSerializer re-declares the SVG namespace on each element; the
      // root <svg> already declares it, so strip the redundant attribute.
      parts.push(
        "  " +
          serializer
            .serializeToString(el)
            .replace(/ xmlns="http:\/\/www\.w3\.org\/2000\/svg"/g, "")
      );
    };
    for (const child of [...root.children]) {
      if (child.tagName === "defs" && !child.hasChildNodes()) continue;
      if (child.tagName === "g" && child.classList.contains("layer")) {
        // Hoist the canvas layer container's children to the top level.
        for (const sub of [...child.children]) push(sub);
        continue;
      }
      push(child);
    }
    const body = parts.join("\n");
    // Re-declare the xlink namespace on the root when any child uses it.
    const xlinkDecl = /xlink:/.test(body)
      ? ' xmlns:xlink="http://www.w3.org/1999/xlink"'
      : "";
    return (
      `<svg xmlns="http://www.w3.org/2000/svg"${xlinkDecl} viewBox="${x} ${y} ${w} ${h}" width="${w}" height="${h}">\n` +
      body +
      "\n</svg>\n"
    );
  } catch {
    return null;
  }
}

/**
 * Advanced SVG editor built on @svgedit/svgcanvas.
 *
 * Replaces the static SVG preview with an interactive canvas that supports
 * path node editing, transforms, grouping, and attribute changes.
 *
 * Props:
 *   svgString     – the current SVG markup to load into the canvas
 *   onComplete    – callback(editedSvgString) invoked when the user finishes
 *   onCancel      – optional callback() invoked when the user aborts
 *   onSaveAsLayer – optional callback(svgString) invoked to snapshot the
 *                   current canvas as a new layer
 *
 * Imperative handle (used by the page-level view controls):
 *   zoomIn() / zoomOut() / zoomFit()
 */
const AdvancedSvgEditor = forwardRef(function AdvancedSvgEditor(
  { svgString, onComplete, onCancel, onSaveAsLayer },
  ref
) {
  const { t } = useI18n();
  const containerRef = useRef(null);
  const canvasRef = useRef(null);
  // Canvas window in user space: the viewBox origin (x, y) plus its size.
  // A fresh trace has x = y = 0; "shrink canvas" moves the origin to crop
  // the canvas without touching the shapes (see doShrinkCanvas).
  const dimsRef = useRef({ x: 0, y: 0, w: 800, h: 600 });
  const [mode, setMode] = useState("select");
  const [canUndo, setCanUndo] = useState(false);
  const [canRedo, setCanRedo] = useState(false);
  const [zoom, setZoom] = useState(1); // 1 = fit to container
  const [containerSize, setContainerSize] = useState({ w: 0, h: 0 });
  // Bumped when the canvas dimensions change (shrink-to-content) so the zoom
  // sizing effect re-runs with the new viewBox.
  const [dimsVersion, setDimsVersion] = useState(0);
  // Latest onSaveAsLayer, so the (stable) imperative handle never goes stale.
  const onSaveAsLayerRef = useRef(onSaveAsLayer);
  useEffect(() => { onSaveAsLayerRef.current = onSaveAsLayer; }, [onSaveAsLayer]);

  // --- Initialize SvgCanvas ---
  useEffect(() => {
    const container = containerRef.current;
    if (!container || !svgString) return;

    // Parse the canvas window from the SVG viewBox (x y w h — the origin
    // may be non-zero after a shrink-to-content).
    const vbMatch = svgString.match(/viewBox="(-?[\d.]+) (-?[\d.]+) ([\d.]+) ([\d.]+)"/);
    const x = vbMatch ? parseFloat(vbMatch[1]) : 0;
    const y = vbMatch ? parseFloat(vbMatch[2]) : 0;
    const w = vbMatch ? parseFloat(vbMatch[3]) : 800;
    const h = vbMatch ? parseFloat(vbMatch[4]) : 600;
    dimsRef.current = { x, y, w, h };

    const canvas = new SvgCanvas(container, {
      canvasName: "advanced-canvas",
      dimensions: [w, h],
      canvas_expansion: 0,
      initFill: { color: "000000", opacity: 1 },
      initStroke: { color: "000000", width: 1, opacity: 1 },
      selectionColor: "#41b6e6",
      wireframe: false,
      showlayers: false,
      no_save_warning: true,
      imgImport: false,
      baseUnit: "px",
      snappingStep: 1,
      gridSnapping: false,
      show_outside_canvas: true,
      selectNew: false,
    });

    // Load the SVG content. Flatten the writer's top-level layer groups so
    // that every shape is a direct child of the svg root and therefore
    // individually selectable (see flattenLayerGroups).
    canvas.setSvgString(flattenLayerGroups(svgString), true);
    canvas.setMode("select");

    // Give the canvas root a viewBox so the artwork can be scaled by
    // resizing the element (see the zoom effect below). The origin (x, y)
    // is preserved so a previously-shrunk canvas keeps its crop.
    canvas.getSvgContent().setAttribute("viewBox", `${x} ${y} ${w} ${h}`);

    // Bind event to track undo/redo availability
    const onHistoryChange = () => {
      setCanUndo(canvas.undoMgr.getUndoStackSize() > 0);
      setCanRedo(canvas.undoMgr.getRedoStackSize() > 0);
    };
    canvas.bind("history", onHistoryChange);

    canvasRef.current = canvas;

    return () => {
      // Note: SvgCanvas has no unbind() — the "history" handler lives on the
      // canvas instance and is garbage-collected with it.
      // Remove the SVG element the canvas created
      const svgEl = container.querySelector("svg");
      if (svgEl) svgEl.remove();
      canvasRef.current = null;
    };
  }, [svgString]);

  // --- Zoom: size the canvas element so the artwork renders at fit * zoom ---
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const measure = () =>
      setContainerSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || containerSize.w <= 0 || containerSize.h <= 0) return;
    const { w, h } = dimsRef.current;
    const availW = Math.max(0, containerSize.w - PAD * 2);
    const availH = Math.max(0, containerSize.h - PAD * 2);
    const fit = Math.min(availW / w, availH / h);
    if (!Number.isFinite(fit) || fit <= 0) return;
    const scale = fit * zoom;
    const nw = Math.round(w * scale * 100) / 100;
    const nh = Math.round(h * scale * 100) / 100;
    const svgEl = canvas.getSvgContent();
    svgEl.setAttribute("width", String(nw));
    svgEl.setAttribute("height", String(nh));
    // The library wraps the content in an outer <svg id="svgroot"> whose
    // width/height are fixed at the original document size. If only the inner
    // content is scaled, the artwork grows/shrinks on window resize while the
    // canvas wrapper stays put, so the two drift out of alignment. Keep the
    // wrapper the same size as the scaled content so canvas and paths scale
    // together.
    const rootEl = canvas.getSvgRoot();
    if (rootEl) {
      rootEl.setAttribute("width", String(nw));
      rootEl.setAttribute("height", String(nh));
      // The checkerboard "canvas" artboard lives on this element (see
      // styles.css). Scale its tile size with the render scale so the grid
      // grows/shrinks in step with the artwork, like Photoshop's transparency
      // grid. A tile is 20 user units, i.e. 20 * scale px on screen, which
      // keeps a constant number of tiles across the artboard at any zoom.
      rootEl.style.setProperty("--adv-s", `${20 * scale}px`);
      // The library appends #selectorParentGroup to #svgroot (a sibling of
      // #svgcontent), so it does NOT inherit the viewBox scaling applied to
      // #svgcontent. Without a compensating transform, selection rectangles
      // are drawn in unscaled user-space coordinates and misalign with the
      // visually-scaled shapes. Apply the same affine mapping the viewBox
      // performs: translate the origin, then scale.
      const { x: vx, y: vy } = dimsRef.current;
      const selGroup = rootEl.querySelector("#selectorParentGroup");
      if (selGroup) {
        selGroup.setAttribute(
          "transform",
          `translate(${-vx * scale}, ${-vy * scale}) scale(${scale})`
        );
      }
    }
  }, [containerSize, zoom, svgString, dimsVersion]);

  useImperativeHandle(
    ref,
    () => ({
      zoomIn: () => setZoom((z) => Math.min(8, z * 1.25)),
      zoomOut: () => setZoom((z) => Math.max(0.1, z / 1.25)),
      zoomFit: () => setZoom(1),
      // Snapshot the canvas as a new layer (used by the layer panel).
      saveAsLayer: () => {
        const canvas = canvasRef.current;
        if (!canvas || !onSaveAsLayerRef.current) return;
        const { x, y, w, h } = dimsRef.current;
        const clean = normalizeEditedSvg(canvas.getSvgString(), w, h, x, y);
        if (clean) onSaveAsLayerRef.current(clean);
      },
    }),
    []
  );

  // --- Tool actions ---
  const switchMode = useCallback((newMode) => {
    setMode(newMode);
    const canvas = canvasRef.current;
    if (canvas) {
      canvas.setMode(newMode);
    }
  }, []);

  const doUndo = useCallback(() => {
    canvasRef.current?.undo();
  }, []);

  const doRedo = useCallback(() => {
    canvasRef.current?.redo();
  }, []);

  const doGroup = useCallback(() => {
    canvasRef.current?.groupSelectedElements();
  }, []);

  const doUngroup = useCallback(() => {
    canvasRef.current?.ungroupSelectedElement();
  }, []);

  const doMoveTop = useCallback(() => {
    canvasRef.current?.moveToTopSelectedElement();
  }, []);

  const doMoveBottom = useCallback(() => {
    canvasRef.current?.moveToBottomSelectedElement();
  }, []);

  const doMoveUp = useCallback(() => {
    canvasRef.current?.moveUpDownSelected("Up");
  }, []);

  const doMoveDown = useCallback(() => {
    canvasRef.current?.moveUpDownSelected("Down");
  }, []);

  const doDelete = useCallback(() => {
    canvasRef.current?.deleteSelectedElements();
  }, []);

  // --- Shrink the canvas to fit the content ---
  // Computes the union bbox of all top-level shapes (in user space) and crops
  // the canvas to it by moving the viewBox origin. The shapes keep their
  // original (image-space) coordinates, so the main editor can still align
  // the vectors with the original image, and shrinking twice stays correct
  // (no cumulative translation). Subsequent "save as layer" / "done" exports
  // carry the offset viewBox.
  const doShrinkCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const content = canvas.getSvgContent();
    let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity;
    let found = false;
    for (const child of [...content.children]) {
      const tag = child.tagName;
      if (tag === "defs" || tag === "title" || tag === "desc") continue;
      let bb;
      try { bb = child.getBBox(); } catch { continue; }
      if (!bb || (bb.width === 0 && bb.height === 0)) continue;
      const ctm = child.getCTM();
      if (!ctm) continue;
      for (const [px, py] of [
        [bb.x, bb.y],
        [bb.x + bb.width, bb.y],
        [bb.x, bb.y + bb.height],
        [bb.x + bb.width, bb.y + bb.height],
      ]) {
        const p = new DOMPoint(px, py).matrixTransform(ctm);
        x1 = Math.min(x1, p.x);
        y1 = Math.min(y1, p.y);
        x2 = Math.max(x2, p.x);
        y2 = Math.max(y2, p.y);
      }
      found = true;
    }
    if (!found) return;

    const pad = 10;
    const newX = x1 - pad;
    const newY = y1 - pad;
    const newW = x2 - x1 + pad * 2;
    const newH = y2 - y1 + pad * 2;

    // Crop by moving the viewBox origin; the shapes are left untouched so
    // their coordinates stay in the original image space. Re-run the zoom
    // sizing with the new canvas window.
    dimsRef.current = { x: newX, y: newY, w: newW, h: newH };
    content.setAttribute("viewBox", `${newX} ${newY} ${newW} ${newH}`);
    setDimsVersion((v) => v + 1);
  }, []);

  // --- Save the current canvas as a new layer ---
  const handleSaveAsLayer = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || !onSaveAsLayer) return;
    const { x, y, w, h } = dimsRef.current;
    const clean = normalizeEditedSvg(canvas.getSvgString(), w, h, x, y);
    if (clean) onSaveAsLayer(clean);
  }, [onSaveAsLayer]);

  // --- Complete / Cancel ---
  const handleComplete = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const { x, y, w, h } = dimsRef.current;
    const clean = normalizeEditedSvg(canvas.getSvgString(), w, h, x, y);
    if (clean) onComplete(clean);
  }, [onComplete]);

  const handleCancel = useCallback(() => {
    if (onCancel) onCancel();
  }, [onCancel]);

  // Keyboard shortcuts
  useEffect(() => {
    const onKey = (e) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key === "z" && !e.shiftKey) { e.preventDefault(); doUndo(); }
      else if (mod && e.key === "z" && e.shiftKey) { e.preventDefault(); doRedo(); }
      else if (mod && e.key === "y") { e.preventDefault(); doRedo(); }
      else if (mod && e.key === "g") { e.preventDefault(); doGroup(); }
      else if (mod && e.shiftKey && (e.key === "G" || e.key === "g")) { e.preventDefault(); doUngroup(); }
      else if (e.key === "Delete" || e.key === "Backspace") {
        // Only delete when not in an input
        if (e.target.tagName !== "INPUT" && e.target.tagName !== "TEXTAREA") {
          e.preventDefault();
          doDelete();
        }
      }
      else if (e.key === "Escape") { handleCancel(); }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [doUndo, doRedo, doGroup, doUngroup, doDelete, handleCancel]);

  return (
    <div className="advanced-editor">
      <AdvancedToolbar
        mode={mode}
        onModeChange={switchMode}
        canUndo={canUndo}
        canRedo={canRedo}
        onUndo={doUndo}
        onRedo={doRedo}
        onGroup={doGroup}
        onUngroup={doUngroup}
        onMoveTop={doMoveTop}
        onMoveBottom={doMoveBottom}
        onMoveUp={doMoveUp}
        onMoveDown={doMoveDown}
        onDelete={doDelete}
        onSaveAsLayer={handleSaveAsLayer}
        onShrinkCanvas={doShrinkCanvas}
        onComplete={handleComplete}
        onCancel={handleCancel}
      />
      <div className="advanced-canvas-container" ref={containerRef} />
    </div>
  );
});

export default AdvancedSvgEditor;

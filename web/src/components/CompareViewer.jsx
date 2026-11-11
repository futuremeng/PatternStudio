import React, { useEffect, useLayoutEffect, useState, useCallback, useRef } from "react";
import { IconCompare, IconZoomIn, IconZoomOut, IconFit } from "./icons.jsx";
import { useI18n } from "../i18n/index.jsx";

/**
 * Viewport interaction shared by the editor preview and the preprocess
 * modal: Ctrl/Cmd + wheel zoom (delegated to `onWheelZoom`) and
 * drag-to-pan when the content overflows. Plain wheel scrolls natively.
 *
 * `enabled` gates the listeners so a mount-once consumer (the editor's
 * advanced-mode viewport) can attach/detach as its viewport mounts.
 */
export function useViewportControls({ viewportRef, enabled = true, onWheelZoom }) {
  const panDragRef = useRef(null); // { startX, startY, scrollLeft, scrollTop }
  const onWheelZoomRef = useRef(onWheelZoom);
  useEffect(() => { onWheelZoomRef.current = onWheelZoom; });

  useEffect(() => {
    if (!enabled) return;
    const el = viewportRef.current;
    if (!el) return;
    const onWheel = (e) => {
      if (!e.ctrlKey && !e.metaKey) return; // let plain wheel scroll
      e.preventDefault();
      onWheelZoomRef.current?.(e.deltaY < 0);
    };
    el.addEventListener("wheel", onWheel, { passive: false });

    // Drag-to-pan when zoomed in
    const onPointerDown = (e) => {
      if (e.button !== 0) return;
      if (el.scrollWidth <= el.clientWidth && el.scrollHeight <= el.clientHeight) return;
      panDragRef.current = {
        startX: e.clientX, startY: e.clientY,
        scrollLeft: el.scrollLeft, scrollTop: el.scrollTop,
      };
      el.style.cursor = "grabbing";
      el.setPointerCapture(e.pointerId);
    };
    const onPointerMove = (e) => {
      const d = panDragRef.current;
      if (!d) return;
      el.scrollLeft = d.scrollLeft - (e.clientX - d.startX);
      el.scrollTop = d.scrollTop - (e.clientY - d.startY);
    };
    const onPointerUp = (e) => {
      if (!panDragRef.current) return;
      panDragRef.current = null;
      el.style.cursor = "";
      el.releasePointerCapture(e.pointerId);
    };
    el.addEventListener("pointerdown", onPointerDown);
    el.addEventListener("pointermove", onPointerMove);
    el.addEventListener("pointerup", onPointerUp);
    el.addEventListener("pointercancel", onPointerUp);

    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("pointermove", onPointerMove);
      el.removeEventListener("pointerup", onPointerUp);
      el.removeEventListener("pointercancel", onPointerUp);
    };
  }, [enabled, viewportRef]);
}

/**
 * Generic image-compare viewer: the original (left) against a result (right),
 * side-by-side or as an overlay with a draggable divider. The right side
 * accepts either an SVG string (trace result) or an image URL (preprocess
 * result). Owns the viewport: measurement, Ctrl/Cmd wheel zoom, drag-to-pan,
 * fit / 1:1 sizing, and the floating control column.
 *
 * The editor passes `extraControls` (patch-select / advanced buttons) and
 * `svgContainerRef` (its selection logic reads the rendered paths from it);
 * the preprocess modal uses the base controls only.
 */
export default function CompareViewer({
  leftUrl,
  right, // { svg: string } | { imageUrl: string } | null
  contentSize, // { w, h } natural size of the content (0/0 until known)
  onImageLoad, // (w, h) — left image's natural size discovered
  compareMode, // "side" | "overlay"
  onCompareModeChange,
  busy, // show the processing badge (and busy placeholder)
  leftLabel,
  rightLabel,
  busyLabel,
  placeholder,
  leftPlaceholder, // shown in the left panel while the image is loading
  extraControls,
  selectMode, // editor: patch-select tool active (pointer events on paths)
  svgContainerRef,
}) {
  const { t } = useI18n();
  const [zoom, setZoom] = useState(1);
  const [actualSize, setActualSize] = useState(false);
  const [overlayPos, setOverlayPos] = useState(50); // divider position, percent
  const [viewportSize, setViewportSize] = useState({ w: 0, h: 0 });
  const viewportRef = useRef(null);
  const overlayRef = useRef(null);
  const overlayDragging = useRef(false);

  useViewportControls({
    viewportRef,
    onWheelZoom: (dir) => {
      setActualSize(false);
      setZoom(z => Math.min(8, Math.max(0.1, z * (dir ? 1.1 : 1 / 1.1))));
    },
  });

  // Track the visible preview size so we can fit the image at zoom = 1 and
  // scale it precisely for the zoom / fit controls. Measured in a layout
  // effect so the first paint already has a correct size.
  useLayoutEffect(() => {
    const el = viewportRef.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      setViewportSize({ w: r.width, h: r.height });
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const zoomIn = useCallback(() => { setActualSize(false); setZoom(z => Math.min(8, z * 1.25)); }, []);
  const zoomOut = useCallback(() => { setActualSize(false); setZoom(z => Math.max(0.1, z / 1.25)); }, []);

  // Overlay compare: move the divider to a pointer x position (percent).
  const updateOverlayPos = useCallback((clientX) => {
    const el = overlayRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0) return;
    const pos = ((clientX - rect.left) / rect.width) * 100;
    setOverlayPos(Math.min(100, Math.max(0, pos)));
  }, []);

  useEffect(() => {
    const onMove = (e) => { if (overlayDragging.current) updateOverlayPos(e.clientX); };
    const onUp = () => { overlayDragging.current = false; };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [updateOverlayPos]);

  // Fit metrics derived from the content's natural size and the viewport.
  const imgW = contentSize?.w || 0;
  const imgH = contentSize?.h || 0;
  const isSide = compareMode === "side";
  const PAD = 16; // matches .preview-stage padding
  const GAP = 16; // matches .before-after gap (fixed, does not scale)
  const availW = Math.max(0, viewportSize.w - PAD * 2);
  const availH = Math.max(0, viewportSize.h - PAD * 2);
  // Scale at which the content exactly fills the viewport (zoom = 1).
  const fitScale = imgW > 0 && imgH > 0 && availW > 0 && availH > 0
    ? (isSide
        ? Math.min(Math.max(0, availW - GAP) / (imgW * 2), availH / imgH)
        : Math.min(availW / imgW, availH / imgH))
    : 0;
  const hasDims = fitScale > 0;
  const ds = hasDims ? fitScale * zoom : 0;
  const boxStyle = hasDims ? { width: imgW * ds, height: imgH * ds } : undefined;
  const imgStyle = hasDims ? boxStyle : { maxHeight: "70vh" };

  const hasRight = !!(right && (right.svg || right.imageUrl));
  const rightBusy = busy && hasRight;
  const rightText = busy ? busyLabel : placeholder;

  // --- Align the vector canvas with the original image (overlay mode) ---
  // A fresh trace's SVG uses viewBox 0 0 imgW imgH, so stretching it to 100%
  // of the compare box lines it up with the image. After the advanced
  // editor's "shrink canvas" the viewBox carries an offset (x y w h) that
  // crops the same image space; the svg element must then be placed at
  // (x*ds, y*ds) with size (w*ds, h*ds). Stretching it to 100% instead would
  // make the browser letterbox the content, and the vectors drift from the
  // image by an amount that grows with the window size / zoom.
  useEffect(() => {
    if (compareMode === "side") return;
    const container = svgContainerRef?.current;
    if (!container || !right?.svg) return;
    const svg = container.querySelector("svg");
    if (!svg) return;
    const vb = svg.viewBox.baseVal;
    if (!vb || vb.width === 0 || vb.height === 0) return;
    const native = vb.x === 0 && vb.y === 0 && vb.width === imgW && vb.height === imgH;
    if (native || !hasDims) {
      // Default CSS sizing (100% of the compare box).
      svg.style.position = "";
      svg.style.left = "";
      svg.style.top = "";
      svg.style.width = "";
      svg.style.height = "";
      return;
    }
    svg.style.position = "absolute";
    svg.style.left = `${vb.x * ds}px`;
    svg.style.top = `${vb.y * ds}px`;
    svg.style.width = `${vb.width * ds}px`;
    svg.style.height = `${vb.height * ds}px`;
  }, [right?.svg, compareMode, ds, hasDims, imgW, imgH, svgContainerRef]);

  const selectCls = selectMode ? " select-mode" : "";

  return (
    <div className="preview-viewport" ref={viewportRef}>
      <div className="preview-stage">
        {compareMode === "side" ? (
          <div className="before-after">
            <div className="ba-panel">
              <span className="label">{leftLabel}</span>
              {leftUrl ? (
                <img
                  src={leftUrl}
                  alt="original"
                  style={imgStyle}
                  onLoad={(e) => onImageLoad?.(e.currentTarget.naturalWidth, e.currentTarget.naturalHeight)}
                />
              ) : <div className="placeholder">{leftPlaceholder ?? placeholder}</div>}
            </div>
            <div className="ba-panel">
              <span className="label">{rightLabel}</span>
              {rightBusy && <span className="busy-badge">{busyLabel}</span>}
              {right?.svg ? (
                <div className={`svg-frame${selectCls}`} ref={svgContainerRef} style={boxStyle} dangerouslySetInnerHTML={{ __html: right.svg }} />
              ) : right?.imageUrl ? (
                <img src={right.imageUrl} alt="result" style={imgStyle} />
              ) : (
                <div className="placeholder" style={boxStyle}>{rightText}</div>
              )}
            </div>
          </div>
        ) : (
          <div className="overlay-wrap">
            {rightBusy && <span className="busy-badge">{busyLabel}</span>}
            {leftUrl && hasRight ? (
              <div
                className="overlay-compare"
                ref={overlayRef}
                style={boxStyle}
                onPointerDown={(e) => {
                  e.preventDefault();
                  overlayDragging.current = true;
                  updateOverlayPos(e.clientX);
                }}
              >
                <img src={leftUrl} alt="original" />
                <div className="overlay-top" style={{ clipPath: `inset(0 0 0 ${overlayPos}%)` }}>
                  {right.svg ? (
                    <div className={`overlay-svg${selectCls}`} ref={svgContainerRef} dangerouslySetInnerHTML={{ __html: right.svg }} />
                  ) : (
                    <img src={right.imageUrl} alt="result" />
                  )}
                </div>
                <span className="overlay-tag overlay-tag-left">{leftLabel}</span>
                <span className="overlay-tag overlay-tag-right">{rightLabel}</span>
                <div className="overlay-divider" style={{ left: `${overlayPos}%` }}>
                  <div className="overlay-handle">‹›</div>
                </div>
              </div>
            ) : (
              <div className="placeholder" style={boxStyle}>{rightText}</div>
            )}
          </div>
        )}
      </div>

      <div className="preview-controls">
        {extraControls}
        <button onClick={() => onCompareModeChange(compareMode === "side" ? "overlay" : "side")} title={t("viewer.toggleCompare")} aria-label={t("viewer.toggleCompare")}>
          <IconCompare />
        </button>
        <button onClick={zoomIn} title={t("editor.zoomIn")} aria-label={t("editor.zoomIn")}>
          <IconZoomIn />
        </button>
        <button onClick={zoomOut} title={t("editor.zoomOut")} aria-label={t("editor.zoomOut")}>
          <IconZoomOut />
        </button>
        <button
          onClick={() => {
            if (actualSize) {
              setActualSize(false);
              setZoom(1);
            } else if (fitScale > 0) {
              setActualSize(true);
              setZoom(1 / fitScale);
            }
          }}
          title={actualSize ? t("editor.fit") : t("viewer.actualSize")}
          aria-label={actualSize ? t("editor.fit") : t("viewer.actualSize")}
        >
          {actualSize ? <IconFit /> : <span style={{ fontSize: "0.65rem", fontWeight: 700 }}>1:1</span>}
        </button>
      </div>
    </div>
  );
}

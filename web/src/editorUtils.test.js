import { describe, it, expect } from "vitest";
import { canvasBox, withCanvasBox, serializeEditedSvg, layerToStandaloneSvg, layerToThumbnailSvg } from "./editorUtils.js";

// A layer snapshot taken after the editing view was cropped to its content:
// the shapes still live in image space, only the canvas box got smaller.
const CROPPED =
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="120 40 300 200" width="300" height="200">' +
  '<path d="M400 60L420 60L400 80Z" fill="#000000"/>' +
  "</svg>";

describe("canvasBox", () => {
  it("pins to the original canvas when its size is known", () => {
    expect(canvasBox(CROPPED, { w: 1000, h: 400 })).toEqual({ x: 0, y: 0, w: 1000, h: 400 });
  });

  it("falls back to the snapshot box when the canvas size is unknown", () => {
    expect(canvasBox(CROPPED, { w: 0, h: 0 })).toEqual({ x: 120, y: 40, w: 300, h: 200 });
    expect(canvasBox(CROPPED, undefined)).toEqual({ x: 120, y: 40, w: 300, h: 200 });
  });

  it("reads a negative origin and defaults to a square without a viewBox", () => {
    expect(canvasBox('<svg viewBox="-20 -10 50 60"></svg>', null)).toEqual({
      x: -20,
      y: -10,
      w: 50,
      h: 60,
    });
    expect(canvasBox("<svg></svg>", null)).toEqual({ x: 0, y: 0, w: 100, h: 100 });
  });
});

describe("withCanvasBox", () => {
  it("restores the original canvas without touching the shapes", () => {
    const out = withCanvasBox(CROPPED, { x: 0, y: 0, w: 1000, h: 400 });
    expect(out).toContain('viewBox="0 0 1000 400"');
    expect(out).toContain('width="1000"');
    expect(out).toContain('height="400"');
    expect(out).toContain('d="M400 60L420 60L400 80Z"');
    expect(out.match(/viewBox=/g)).toHaveLength(1);
    expect(out.match(/width=/g)).toHaveLength(1);
    expect(out.match(/height=/g)).toHaveLength(1);
  });

  it("adds the box to a root that declares none", () => {
    const out = withCanvasBox('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0Z"/></svg>', {
      x: 0,
      y: 0,
      w: 64,
      h: 64,
    });
    expect(out).toContain('viewBox="0 0 64 64"');
    expect(out).toContain("<path");
  });

  it("keeps a self-closing root self-closing", () => {
    const out = withCanvasBox('<svg viewBox="0 0 10 10"/>', { x: 0, y: 0, w: 20, h: 20 });
    expect(out).toBe('<svg viewBox="0 0 20 20" width="20" height="20"/>');
  });
});

describe("layerToStandaloneSvg", () => {
  it("emits the layer on the original canvas, shapes untouched", () => {
    const out = layerToStandaloneSvg(CROPPED, { w: 1000, h: 400 });
    expect(out).toContain('viewBox="0 0 1000 400"');
    expect(out).toContain('width="1000"');
    expect(out).toContain('height="400"');
    expect(out).toContain('d="M400 60L420 60L400 80Z"');
    expect(out.match(/viewBox=/g)).toHaveLength(1);
  });

  it("adds the SVG namespace to a snapshot that lacks one", () => {
    const out = layerToStandaloneSvg('<svg><path d="M0 0Z"/></svg>', { w: 64, h: 64 });
    expect(out.match(/xmlns="/g)).toHaveLength(1);
    expect(out).toContain('xmlns="http://www.w3.org/2000/svg"');
  });

  it("thumbnail and export use the same canvas", () => {
    const box = { w: 1000, h: 400 };
    const thumb = layerToThumbnailSvg(CROPPED, box);
    expect(thumb).toContain('viewBox="0 0 1000 400"');
    expect(thumb).toContain('d="M400 60L420 60L400 80Z"');
    expect(thumb.match(/viewBox=/g)).toHaveLength(1);
  });
});

describe("serializeEditedSvg", () => {
  const makeSvg = () => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", "M10 10Z");
    path.setAttribute("style", "stroke:#41b6e6"); // leftover selection highlight
    svg.appendChild(path);
    return svg;
  };

  it("emits the snapshot on the original canvas", () => {
    const out = serializeEditedSvg(makeSvg(), { w: 1000, h: 400 });
    expect(out).toContain('viewBox="0 0 1000 400"');
    expect(out).toContain('width="1000"');
    expect(out).toContain('height="400"');
    expect(out).toContain('<g id="edited-layer" data-role="edited">');
    expect(out).toContain("<path");
    expect(out).not.toContain("stroke:#41b6e6");
  });
});

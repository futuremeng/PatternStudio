import { describe, it, expect } from "vitest";
import {
  DEFAULT_PARAMS,
  VECTORIZE_PARAM_KEYS,
  LAYER_PARAM_KEYS,
  MODE_PRESETS,
  splitTraceParams,
  mergeTraceParams,
  presetFullParams,
  familyFromParams,
  TraceFlowError,
  newTraceRequestId,
} from "./api.js";

describe("param key groups", () => {
  it("lineArt is grouped with vectorization (frontend UX split)", () => {
    expect(VECTORIZE_PARAM_KEYS).toContain("lineArt");
    expect(LAYER_PARAM_KEYS).not.toContain("lineArt");
  });

  it("layer keys are exactly the layering-stage params", () => {
    expect(LAYER_PARAM_KEYS).toEqual(["layers", "layerStrategy", "complexityThreshold"]);
  });

  it("vectorize and layer key sets are disjoint", () => {
    const overlap = VECTORIZE_PARAM_KEYS.filter((k) => LAYER_PARAM_KEYS.includes(k));
    expect(overlap).toEqual([]);
  });

  it("every default param is covered by exactly one group", () => {
    const all = [...VECTORIZE_PARAM_KEYS, ...LAYER_PARAM_KEYS];
    for (const k of Object.keys(DEFAULT_PARAMS)) {
      expect(all).toContain(k);
    }
  });
});

describe("splitTraceParams", () => {
  it("splits a full set into trace + layer parts", () => {
    const { trace, layer } = splitTraceParams({ ...DEFAULT_PARAMS, layers: 3, lineArt: true });
    for (const k of VECTORIZE_PARAM_KEYS) expect(trace).toHaveProperty(k);
    for (const k of LAYER_PARAM_KEYS) expect(layer).toHaveProperty(k);
    expect(trace.lineArt).toBe(true);
    expect(layer.layers).toBe(3);
  });

  it("fills missing keys from DEFAULT_PARAMS", () => {
    const { trace, layer } = splitTraceParams({ layers: 5 });
    expect(trace.preset).toBe(DEFAULT_PARAMS.preset);
    expect(trace.clustering).toBe(DEFAULT_PARAMS.clustering);
    expect(layer.layers).toBe(5);
    expect(layer.layerStrategy).toBe(DEFAULT_PARAMS.layerStrategy);
  });

  it("handles null/undefined input as defaults", () => {
    const { trace, layer } = splitTraceParams(null);
    expect(trace).toEqual(
      Object.fromEntries(VECTORIZE_PARAM_KEYS.map((k) => [k, DEFAULT_PARAMS[k]]))
    );
    expect(layer).toEqual(
      Object.fromEntries(LAYER_PARAM_KEYS.map((k) => [k, DEFAULT_PARAMS[k]]))
    );
  });
});

describe("mergeTraceParams", () => {
  it("round-trips with splitTraceParams", () => {
    const full = { ...DEFAULT_PARAMS, layers: 4, layerStrategy: "color", lineArt: true, simplify: 0.3 };
    const { trace, layer } = splitTraceParams(full);
    const merged = mergeTraceParams(trace, layer);
    for (const k of Object.keys(DEFAULT_PARAMS)) expect(merged[k]).toBe(full[k]);
  });

  it("defaults missing parts", () => {
    const merged = mergeTraceParams(null, null);
    expect(merged).toEqual(DEFAULT_PARAMS);
  });

  it("layer overrides win over trace on conflict", () => {
    const merged = mergeTraceParams({ preset: "bw" }, { layers: 2 });
    expect(merged.preset).toBe("bw");
    expect(merged.layers).toBe(2);
  });
});

describe("presetFullParams", () => {
  it("merges preset overrides onto defaults", () => {
    const p = presetFullParams("lineArt");
    expect(p.preset).toBe("bw");
    expect(p.clustering).toBe("binary");
    expect(p.lineArt).toBe(true);
    expect(p.mode).toBe("spline");
    // Untouched keys keep their defaults.
    expect(p.colorPrecision).toBe(DEFAULT_PARAMS.colorPrecision);
  });

  it("returns defaults for an unknown preset", () => {
    expect(presetFullParams("doesNotExist")).toEqual(DEFAULT_PARAMS);
  });

  it("every preset maps to a consistent family", () => {
    expect(familyFromParams(presetFullParams("lineArt"))).toBe("lineArt");
    expect(familyFromParams(presetFullParams("bw"))).toBe("bw");
    expect(familyFromParams(presetFullParams("lowPoly"))).toBe("lowPoly");
    expect(familyFromParams(presetFullParams("gradient"))).toBe("gradient");
    expect(familyFromParams(presetFullParams("color"))).toBe("color");
  });
});

describe("familyFromParams", () => {
  it("derives the family by priority (lineArt > bw > lowPoly > gradient > color)", () => {
    expect(familyFromParams({ lineArt: true, clustering: "binary" })).toBe("lineArt");
    expect(familyFromParams({ clustering: "binary" })).toBe("bw");
    expect(familyFromParams({ mode: "polygon" })).toBe("lowPoly");
    expect(familyFromParams({ preset: "photo" })).toBe("gradient");
    expect(familyFromParams({ preset: "poster", clustering: "color" })).toBe("color");
  });

  it("lineArt requires binary clustering", () => {
    // lineArt with color clustering falls through to the color family.
    expect(familyFromParams({ lineArt: true, clustering: "color" })).toBe("color");
  });

  it("defaults to color for null/empty", () => {
    expect(familyFromParams(null)).toBe("color");
    expect(familyFromParams(undefined)).toBe("color");
    expect(familyFromParams({})).toBe("color");
  });
});

describe("TraceFlowError", () => {
  it("is an Error with a code and name", () => {
    const e = new TraceFlowError("cancelled", "trace cancelled");
    expect(e).toBeInstanceOf(Error);
    expect(e).toBeInstanceOf(TraceFlowError);
    expect(e.name).toBe("TraceFlowError");
    expect(e.code).toBe("cancelled");
    expect(e.message).toBe("trace cancelled");
  });

  it("supports the busy code", () => {
    const e = new TraceFlowError("busy", "trace busy");
    expect(e.code).toBe("busy");
  });
});

describe("newTraceRequestId", () => {
  it("returns a non-empty string", () => {
    expect(typeof newTraceRequestId()).toBe("string");
    expect(newTraceRequestId().length).toBeGreaterThan(0);
  });

  it("returns unique ids across calls", () => {
    const ids = new Set(Array.from({ length: 100 }, () => newTraceRequestId()));
    expect(ids.size).toBe(100);
  });
});

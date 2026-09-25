import { describe, it, expect } from "vitest";
import { stableStringify, makeKey, makeVectorizeKey } from "./traceCache.js";

describe("stableStringify", () => {
  it("serializes null/undefined as 'null'", () => {
    expect(stableStringify(null)).toBe("null");
    expect(stableStringify(undefined)).toBe("null");
  });

  it("serializes primitives", () => {
    expect(stableStringify("abc")).toBe('"abc"');
    expect(stableStringify(42)).toBe("42");
    expect(stableStringify(0.5)).toBe("0.5");
    expect(stableStringify(true)).toBe("true");
    expect(stableStringify(false)).toBe("false");
  });

  it("serializes arrays in order", () => {
    expect(stableStringify([1, 2, 3])).toBe("[1,2,3]");
    expect(stableStringify([])).toBe("[]");
    expect(stableStringify([1, "a", null])).toBe('[1,"a",null]');
  });

  it("sorts object keys recursively (deterministic)", () => {
    // Key order in the source object must not matter.
    const a = { b: 1, a: 2, c: { y: 1, x: 2 } };
    const b = { a: 2, c: { x: 2, y: 1 }, b: 1 };
    expect(stableStringify(a)).toBe(stableStringify(b));
    expect(stableStringify(a)).toBe('{"a":2,"b":1,"c":{"x":2,"y":1}}');
  });

  it("distinguishes different values", () => {
    expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 2 }));
    expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 1, b: 0 }));
  });
});

describe("makeKey", () => {
  it("combines imageId and params", () => {
    expect(makeKey("img1", { a: 1 })).toBe('img1::{"a":1}');
  });

  it("treats null/undefined params as empty object", () => {
    expect(makeKey("img1", null)).toBe("img1::{}");
    expect(makeKey("img1", undefined)).toBe("img1::{}");
  });

  it("is insensitive to param key order but sensitive to values", () => {
    expect(makeKey("img1", { a: 1, b: 2 })).toBe(makeKey("img1", { b: 2, a: 1 }));
    expect(makeKey("img1", { a: 1 })).not.toBe(makeKey("img1", { a: 2 }));
    expect(makeKey("img1", { a: 1 })).not.toBe(makeKey("img2", { a: 1 }));
  });
});

describe("makeVectorizeKey", () => {
  it("uses the vec tag and trace params", () => {
    expect(makeVectorizeKey("img1", { a: 1 })).toBe('img1::vec::{"a":1}');
  });

  it("is distinct from the processed-result key", () => {
    expect(makeVectorizeKey("img1", { a: 1 })).not.toBe(makeKey("img1", { a: 1 }));
  });
});

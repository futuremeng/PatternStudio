import { describe, it, expect } from "vitest";
import { buildLineageTree, countTreeNodes } from "./lineageTree.js";

// Entry factory: { file, root, parent, kind, confirmed, confirmedFile, createdAt, meta }
const entry = (over = {}) => ({
  file: "gen.png",
  root: "cat.png",
  parent: "cat.png",
  kind: "enhance",
  confirmed: false,
  confirmedFile: null,
  createdAt: "2026-01-01T00:00:00.000Z",
  meta: null,
  ...over,
});

describe("buildLineageTree", () => {
  it("lists originals as rows with no derived nodes", () => {
    const rows = buildLineageTree(["cat.png", "dog.png"], [], "confirmed");
    expect(rows.map(r => r.original)).toEqual(["cat.png", "dog.png"]);
    expect(rows.every(r => r.nodes.length === 0)).toBe(true);
    expect(rows.every(r => !r.missing)).toBe(true);
  });

  it("hangs confirmed entries under their root original", () => {
    const lineage = [
      entry({ file: "cat_enhanced_1.png", confirmed: true, confirmedFile: "cat_enhanced_1.png" }),
    ];
    const rows = buildLineageTree(["cat.png", "cat_enhanced_1.png"], lineage, "confirmed");
    // The confirmed file is not an original row; it is a node under cat.png.
    expect(rows.map(r => r.original)).toEqual(["cat.png"]);
    expect(rows[0].nodes.map(n => n.name)).toEqual(["cat_enhanced_1.png"]);
    expect(rows[0].nodes[0].openName).toBe("cat_enhanced_1.png");
  });

  it("confirmed filter hides auto-generated previews", () => {
    const lineage = [
      entry({ file: "gen_1.png" }), // unconfirmed
      entry({ file: "cat_enhanced_1.png", confirmed: true, confirmedFile: "cat_enhanced_1.png" }),
    ];
    const rows = buildLineageTree(["cat.png", "cat_enhanced_1.png"], lineage, "confirmed");
    expect(rows[0].nodes.map(n => n.name)).toEqual(["cat_enhanced_1.png"]);
  });

  it("all filter shows auto-generated previews, openable in the editor", () => {
    const lineage = [
      entry({ file: "gen_1.png", createdAt: "2026-01-02T00:00:00.000Z" }),
      entry({ file: "cat_enhanced_1.png", confirmed: true, confirmedFile: "cat_enhanced_1.png" }),
    ];
    const rows = buildLineageTree(["cat.png", "cat_enhanced_1.png"], lineage, "all");
    const names = rows[0].nodes.map(n => n.name);
    expect(names).toContain("gen_1.png");
    expect(names).toContain("cat_enhanced_1.png");
    const gen = rows[0].nodes.find(n => n.name === "gen_1.png");
    expect(gen.openName).toBe("gen_1.png"); // auto-generated images open too
    expect(gen.confirmed).toBe(false);
  });

  it("chains nodes by parent (generated parent visible in all mode)", () => {
    const lineage = [
      entry({ file: "gen_1.png", createdAt: "2026-01-01T00:00:00.000Z" }),
      entry({
        file: "gen_2.png", parent: "gen_1.png",
        createdAt: "2026-01-02T00:00:00.000Z",
      }),
    ];
    const rows = buildLineageTree(["cat.png"], lineage, "all");
    expect(rows[0].nodes).toHaveLength(1);
    expect(rows[0].nodes[0].name).toBe("gen_1.png");
    expect(rows[0].nodes[0].children.map(c => c.name)).toEqual(["gen_2.png"]);
    expect(countTreeNodes(rows[0].nodes)).toBe(2);
  });

  it("re-attaches to the root when the parent is hidden (confirmed mode)", () => {
    const lineage = [
      entry({ file: "gen_1.png" }),
      entry({
        file: "gen_2.png", parent: "gen_1.png",
        confirmed: true, confirmedFile: "cat_enhanced_1.png",
        createdAt: "2026-01-02T00:00:00.000Z",
      }),
    ];
    const rows = buildLineageTree(["cat.png", "cat_enhanced_1.png"], lineage, "confirmed");
    // gen_1 is hidden, so the confirmed child hangs directly off the root.
    expect(rows[0].nodes.map(n => n.name)).toEqual(["cat_enhanced_1.png"]);
  });

  it("adds a missing-original row when the root file is gone", () => {
    const lineage = [
      entry({ file: "cat_enhanced_1.png", confirmed: true, confirmedFile: "cat_enhanced_1.png" }),
    ];
    const rows = buildLineageTree(["cat_enhanced_1.png"], lineage, "confirmed");
    expect(rows).toHaveLength(1);
    expect(rows[0].missing).toBe(true);
    expect(rows[0].original).toBe("cat.png");
  });

  it("sorts nodes by creation time", () => {
    const lineage = [
      entry({ file: "b.png", confirmed: true, confirmedFile: "b.png", createdAt: "2026-01-03T00:00:00.000Z" }),
      entry({ file: "a.png", confirmed: true, confirmedFile: "a.png", createdAt: "2026-01-01T00:00:00.000Z" }),
    ];
    const rows = buildLineageTree(["cat.png", "a.png", "b.png"], lineage, "confirmed");
    expect(rows[0].nodes.map(n => n.name)).toEqual(["a.png", "b.png"]);
  });

  it("tolerates null lineage and empty images", () => {
    expect(buildLineageTree([], null, "all")).toEqual([]);
    expect(buildLineageTree(null, null, "confirmed")).toEqual([]);
  });
});
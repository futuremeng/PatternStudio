import React, { useMemo, useState } from "react";
import { downloadSvg } from "./api.js";

/**
 * Layer editor: parses the layered SVG (<g id data-role> groups) and allows
 * toggling visibility, recoloring, and deleting layers. Downloads the
 * modified SVG.
 */
export default function LayerEditor({ svg, width, height, stats }) {
  const [doc, setDoc] = useState(() => new DOMParser().parseFromString(svg, "image/svg+xml"));
  const [version, setVersion] = useState(0);

  const layers = useMemo(() => {
    const groups = Array.from(doc.documentElement.querySelectorAll(":scope > g"));
    return groups.map((g, i) => ({
      el: g,
      id: g.id || `layer-${i}`,
      role: g.dataset.role || "",
      shapes: g.querySelectorAll("path").length,
      visible: g.style.display !== "none",
    }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [doc, version]);

  function mutate(fn) {
    fn(doc);
    setVersion((v) => v + 1);
  }

  function toggleVisible(el) {
    mutate((d) => {
      const g = d.querySelector(`g[id="${el.id}"]`);
      if (g) g.style.display = g.style.display === "none" ? "" : "none";
    });
  }

  function recolor(el, color) {
    mutate((d) => {
      const g = d.querySelector(`g[id="${el.id}"]`);
      if (!g) return;
      g.querySelectorAll("path").forEach((p) => p.setAttribute("fill", color));
    });
  }

  function removeLayer(el) {
    mutate((d) => {
      const g = d.querySelector(`g[id="${el.id}"]`);
      if (g) g.remove();
    });
  }

  function download() {
    downloadSvg(new XMLSerializer().serializeToString(doc.documentElement), "patternstudio.svg");
  }

  return (
    <div className="split">
      <aside className="sidebar">
        <div className="layer-panel">
          <div className="layer-head">
            <h3>图层 ({layers.length})</h3>
            <span className="muted">
              {stats.shapes} shapes · {width}×{height}
            </span>
          </div>
          <ul className="layer-list">
            {layers.map((l) => (
              <li key={l.id} className={l.visible ? "" : "hidden"}>
                <button
                  className="eye"
                  title={l.visible ? "隐藏" : "显示"}
                  onClick={() => toggleVisible(l.el)}
                >
                  {l.visible ? "👁" : "🚫"}
                </button>
                <span className="layer-name" title={l.role}>
                  {l.id}
                </span>
                <span className="muted">{l.shapes}</span>
                <input
                  type="color"
                  title="重新着色"
                  defaultValue="#ff0000"
                  onChange={(e) => recolor(l.el, e.target.value)}
                />
                <button className="del" title="删除图层" onClick={() => removeLayer(l.el)}>
                  ✕
                </button>
              </li>
            ))}
            {layers.length === 0 && <li className="muted">（无图层）</li>}
          </ul>
          <button className="primary" onClick={download}>
            下载 SVG
          </button>
        </div>
      </aside>

      <section className="content preview">
        <div
          className="svg-frame"
          dangerouslySetInnerHTML={{ __html: new XMLSerializer().serializeToString(doc.documentElement) }}
        />
      </section>
    </div>
  );
}

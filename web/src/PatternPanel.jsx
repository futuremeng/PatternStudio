import React, { useEffect, useState } from "react";
import { downloadSvg, generatePattern, listPatterns } from "./api.js";

export default function PatternPanel() {
  const [kinds, setKinds] = useState([]);
  const [params, setParams] = useState({ kind: "rosette", n_units: 8, complexity: 50, size: 400 });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [svg, setSvg] = useState(null);

  useEffect(() => {
    listPatterns()
      .then((d) => setKinds(d.patterns || []))
      .catch((e) => setError(`加载纹样列表失败: ${e.message}`));
  }, []);

  const set = (key, num = false) => (e) =>
    setParams((p) => ({ ...p, [key]: num ? Number(e.target.value) : e.target.value }));

  async function onGenerate() {
    setBusy(true);
    setError(null);
    try {
      const res = await generatePattern(params);
      setSvg(res.svg);
    } catch (e) {
      setError(`生成失败: ${e.message}`);
    } finally {
      setBusy(false);
    }
  }

  function download() {
    downloadSvg(svg, `${params.kind}.svg`);
  }

  return (
    <div className="split">
      <aside className="sidebar">
        <div className="params">
          <label>
            纹样
            <select value={params.kind} onChange={set("kind")}>
              {kinds.map((k) => (
                <option key={k.kind} value={k.kind}>
                  {k.kind} — {k.description}
                </option>
              ))}
            </select>
          </label>
          <label>
            重复单元数
            <input type="number" min="1" max="32" value={params.n_units} onChange={set("n_units", true)} />
          </label>
          <label>
            复杂度 (0-100)
            <input
              type="range"
              min="0"
              max="100"
              value={params.complexity}
              onChange={set("complexity", true)}
            />
            <span className="muted">{params.complexity}</span>
          </label>
          <label>
            尺寸 (px)
            <input type="number" min="50" max="2000" step="50" value={params.size} onChange={set("size", true)} />
          </label>
          <button className="primary" disabled={busy} onClick={onGenerate}>
            {busy ? "生成中..." : "生成"}
          </button>
          {svg && (
            <button onClick={download}>下载 SVG</button>
          )}
          {error && <div className="error">{error}</div>}
        </div>
      </aside>
      <section className="content preview">
        {svg ? (
          <div className="svg-frame" dangerouslySetInnerHTML={{ __html: svg }} />
        ) : (
          <div className="placeholder">{busy ? "正在生成..." : "选择纹样参数并点击生成"}</div>
        )}
      </section>
    </div>
  );
}

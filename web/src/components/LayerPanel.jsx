import React, { useState } from "react";
import { downloadSvg } from "../api.js";

const PROFILE_TITLES = {
  L0: "L0 粗：背景层 — 加大简化容差 + 降低坐标精度",
  L1: "L1 标准：保持理想参数结果",
  L2: "L2 细：细节层 — 更高坐标精度 + 冗余段清理",
};

function LayerCard({ layer, selected, onToggle }) {
  return (
    <div
      className={`layer-card ${selected ? "selected" : ""}`}
      onClick={onToggle}
    >
      <input
        type="checkbox"
        checked={selected}
        onChange={onToggle}
        onClick={e => e.stopPropagation()}
      />
      <div className="mini-preview" dangerouslySetInnerHTML={{ __html: layer.previewSvg }} />
      <div className="info">
        <div className="name">
          {layer.name}
          {layer.profile && (
            <span className="profile-badge" title={PROFILE_TITLES[layer.profile] || layer.profile}>
              {layer.profile}
            </span>
          )}
        </div>
        <div className="meta">
          <span className="color-dot" style={{ background: layer.dominantColor }} />
          <span>{layer.shapeCount} shapes</span>
          <span>·</span>
          <span>{layer.role}</span>
        </div>
      </div>
    </div>
  );
}

export default function LayerPanel({ layers, strategyInfo }) {
  const [selected, setSelected] = useState(new Set());

  const toggle = (name) => {
    setSelected(prev => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const selectAll = () => setSelected(new Set(layers.map(l => l.name)));
  const selectNone = () => setSelected(new Set());

  const exportSelected = () => {
    if (selected.size === 0) return;
    // Build SVG from selected layers' preview SVGs
    const selectedLayers = layers.filter(l => selected.has(l.name));
    const svgParts = selectedLayers.map(l => l.previewSvg);
    // Combine into a single SVG (simplified: just concatenate the path elements)
    const combined = svgParts.map(s => {
      // Extract content between <svg> and </svg>
      const match = s.match(/<svg[^>]*>(.*)<\/svg>/s);
      return match ? match[1] : "";
    }).join("\n");
    const width = selectedLayers[0]?.previewSvg?.match(/viewBox="0 0 (\d+) (\d+)"/);
    const w = width ? width[1] : "100";
    const h = width ? width[2] : "100";
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${w}" width="${w}" height="${h}">${combined}</svg>`;
    downloadSvg(svg, "selected-layers.svg");
  };

  if (layers.length === 0) {
    return <div className="muted">暂无图层数据（请先 Trace）</div>;
  }

  return (
    <div className="layer-panel">
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <h3>图层 ({layers.length})</h3>
        <div style={{ display: "flex", gap: "0.3rem" }}>
          <button onClick={selectAll} style={{ fontSize: "0.75rem", padding: "0.2rem 0.5rem" }}>全选</button>
          <button onClick={selectNone} style={{ fontSize: "0.75rem", padding: "0.2rem 0.5rem" }}>清空</button>
        </div>
      </div>

      {strategyInfo?.reason && (
        <div className="strategy-note" title={`策略: ${strategyInfo.strategy} · 层数: ${strategyInfo.layers} · 复杂度阈值: ${strategyInfo.complexityThreshold}%`}>
          {strategyInfo.reason}
        </div>
      )}

      {layers.map(layer => (
        <LayerCard
          key={layer.name}
          layer={layer}
          selected={selected.has(layer.name)}
          onToggle={() => toggle(layer.name)}
        />
      ))}

      <button
        className="primary"
        onClick={exportSelected}
        disabled={selected.size === 0}
      >
        导出选中图层 ({selected.size})
      </button>
    </div>
  );
}
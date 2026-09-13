import React, { useState } from "react";
import { downloadSvg } from "../api.js";
import { useI18n } from "../i18n/index.jsx";

function LayerCard({ layer, selected, onToggle }) {
  const { t } = useI18n();
  const PROFILE_TITLES = {
    L0: t("layer.profileL0"),
    L1: t("layer.profileL1"),
    L2: t("layer.profileL2"),
  };
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
          <span>{t("layer.shapes", { n: layer.shapeCount })}</span>
          <span>·</span>
          <span>{layer.role}</span>
        </div>
      </div>
    </div>
  );
}

export default function LayerPanel({ layers, strategyInfo, profileMs }) {
  const { t } = useI18n();
  const [selected, setSelected] = useState(new Set());

  const STRATEGY_LABELS = {
    none: t("layer.strategyNone"),
    color: t("layer.strategyColor"),
    complexity: t("layer.strategyComplexity"),
    hybrid: t("layer.strategyHybrid"),
  };

  // The backend sends an i18n key (reasonKey); legacy cached results carry a
  // pre-built Chinese `reason` string instead — fall back to it.
  const strategyReason = strategyInfo?.reasonKey
    ? t(`reason.${strategyInfo.reasonKey}`, {
        c: strategyInfo.distinctColors,
        budget: strategyInfo.detailBudget,
      })
    : strategyInfo?.reason;

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
    return <div className="muted">{t("layer.noData")}</div>;
  }

  return (
    <div className="layer-panel">
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between" }}>
        <h3>{t("layer.title", { n: layers.length })}</h3>
        <div style={{ display: "flex", gap: "0.3rem" }}>
          <button onClick={selectAll} style={{ fontSize: "0.75rem", padding: "0.2rem 0.5rem" }}>{t("layer.selectAll")}</button>
          <button onClick={selectNone} style={{ fontSize: "0.75rem", padding: "0.2rem 0.5rem" }}>{t("layer.clear")}</button>
        </div>
      </div>

      {strategyReason && (
        <div className="strategy-note">
          <div className="strategy-header">
            <span className="strategy-icon">⚙</span>
            <span className="strategy-type">{STRATEGY_LABELS[strategyInfo.strategy] || strategyInfo.strategy}</span>
            <span className="strategy-meta" title={t("layer.metaTitle")}>
              {t("layer.meta", {
                layers: strategyInfo.layers,
                colors: strategyInfo.distinctColors,
                budget: strategyInfo.detailBudget,
              })}
              {profileMs != null && ` · ${profileMs}ms`}
            </span>
          </div>
          <div className="strategy-reason">{strategyReason}</div>
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
        {t("layer.exportSelected", { n: selected.size })}
      </button>
    </div>
  );
}
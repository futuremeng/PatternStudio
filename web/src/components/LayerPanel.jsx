import React, { useState } from "react";
import JSZip from "jszip";
import { downloadSvg, downloadBlob } from "../api.js";
import { useI18n } from "../i18n/index.jsx";

// Rebuild a layer's preview SVG as a standalone file with explicit width/height.
function layerToStandaloneSvg(previewSvg) {
  const innerMatch = previewSvg.match(/<svg[^>]*>([\s\S]*)<\/svg>/);
  const inner = innerMatch ? innerMatch[1] : "";
  const vbMatch = previewSvg.match(/viewBox="0 0 ([\d.]+) ([\d.]+)"/);
  const w = vbMatch ? vbMatch[1] : "100";
  const h = vbMatch ? vbMatch[2] : "100";
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${w} ${h}" width="${w}" height="${h}">${inner}</svg>\n`;
}

// Make a layer name safe to use as a file name inside the export ZIP.
function sanitizeFilename(name) {
  return name.replace(/[\\/:*?"<>|]/g, "_").trim() || "layer";
}

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

export default function LayerPanel({ layers, strategyInfo, profileMs, onSaveAsLayer, canSaveAsLayer }) {
  const { t } = useI18n();
  const [selected, setSelected] = useState(new Set());
  const [exporting, setExporting] = useState(false);

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

  // Export the selected layers: a single layer downloads its own .svg directly,
  // while multiple layers are bundled (each as its own .svg) into one ZIP.
  const exportSelected = async () => {
    if (selected.size === 0 || exporting) return;
    const selectedLayers = layers.filter(l => selected.has(l.name));
    setExporting(true);
    try {
      if (selectedLayers.length === 1) {
        const layer = selectedLayers[0];
        await downloadSvg(layerToStandaloneSvg(layer.previewSvg), `${sanitizeFilename(layer.name)}.svg`);
        return;
      }
      const zip = new JSZip();
      for (const layer of selectedLayers) {
        zip.file(`${sanitizeFilename(layer.name)}.svg`, layerToStandaloneSvg(layer.previewSvg));
      }
      const blob = await zip.generateAsync({ type: "blob" });
      await downloadBlob(blob, "selected-layers.zip");
    } finally {
      setExporting(false);
    }
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

      {canSaveAsLayer && (
        <button
          className="save-layer-btn"
          onClick={onSaveAsLayer}
          title={t("layer.saveAsLayerHint")}
        >
          {t("layer.saveAsLayer")}
        </button>
      )}

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
        disabled={selected.size === 0 || exporting}
      >
        {t("layer.exportSelected", { n: selected.size })}
      </button>
    </div>
  );
}
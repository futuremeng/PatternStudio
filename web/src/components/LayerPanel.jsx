import React, { useState } from "react";
import JSZip from "jszip";
import { downloadSvg, downloadBlob } from "../api.js";
import { useI18n } from "../i18n/index.jsx";
import { canvasBox, withCanvasBox } from "../editorUtils.js";
import { SegmentedControl, Slider, Group } from "./ParameterPanel.jsx";

// Rebuild a layer's preview SVG as a standalone file with explicit width/height
// on the *original* canvas. A separated layer holds a subset of the shapes, not
// a crop: the shapes keep their image-space coordinates, so the canvas must
// stay the size of the source image (`canvasSize`). Only when that is unknown
// does the box carried by the snapshot itself get used — a layer snapshotted
// from a shrink-to-content editing canvas would otherwise export shrunken.
function layerToStandaloneSvg(previewSvg, canvasSize) {
  const innerMatch = previewSvg.match(/<svg[^>]*>([\s\S]*)<\/svg>/);
  const inner = innerMatch ? innerMatch[1] : "";
  const box = canvasBox(previewSvg, canvasSize);
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${box.x} ${box.y} ${box.w} ${box.h}" width="${box.w}" height="${box.h}">${inner}</svg>\n`;
}

// The card thumbnail shows the layer in the context of the full canvas, so the
// figure's position within the original artwork stays recognizable.
function layerToThumbnailSvg(previewSvg, canvasSize) {
  return withCanvasBox(previewSvg, canvasBox(previewSvg, canvasSize));
}

// Make a layer name safe to use as a file name inside the export ZIP.
function sanitizeFilename(name) {
  return name.replace(/[\\/:*?"<>|]/g, "_").trim() || "layer";
}

function LayerCard({ layer, selected, onToggle, canvasSize }) {
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
      <div className="mini-preview" dangerouslySetInnerHTML={{ __html: layerToThumbnailSvg(layer.previewSvg, canvasSize) }} />
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

export default function LayerPanel({
  layers, strategyInfo, profileMs, onSaveAsLayer, canSaveAsLayer,
  layerParams, onLayerParamsChange, onProcess, processBusy, processPending, processed,
  canvasSize,
}) {
  const { t } = useI18n();
  const [selected, setSelected] = useState(new Set());
  const [exporting, setExporting] = useState(false);
  const set = (key) => (val) => onLayerParamsChange(prev => ({ ...prev, [key]: val }));

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
        await downloadSvg(layerToStandaloneSvg(layer.previewSvg, canvasSize), `${sanitizeFilename(layer.name)}.svg`);
        return;
      }
      const zip = new JSZip();
      for (const layer of selectedLayers) {
        zip.file(`${sanitizeFilename(layer.name)}.svg`, layerToStandaloneSvg(layer.previewSvg, canvasSize));
      }
      const blob = await zip.generateAsync({ type: "blob" });
      await downloadBlob(blob, "selected-layers.zip");
    } finally {
      setExporting(false);
    }
  };

  return (
    <div className="layer-panel">
      {/* Layering settings: applied to the cached vectorized document via the
          "Process" button below — changing them never re-vectorizes. */}
      <Group title={t("param.groupLayerStrategy")}>
        <SegmentedControl
          label={t("param.layerStrategy")}
          title={t("param.layerStrategyTitle")}
          value={layerParams.layerStrategy ?? "none"}
          options={[
            { value: "none", label: t("param.singleLayer"), title: t("param.singleLayerTitle") },
            { value: "auto", label: t("param.auto"), title: t("param.autoStrategyTitle") },
            { value: "color", label: t("param.color"), title: t("param.colorStrategyTitle") },
            { value: "spatial", label: t("param.spatial"), title: t("param.spatialStrategyTitle") },
            { value: "complexity", label: t("param.complexity"), title: t("param.complexityStrategyTitle") },
            { value: "hybrid", label: t("param.hybrid"), title: t("param.hybridStrategyTitle") },
          ]}
          onChange={set("layerStrategy")}
        />
        {layerParams.layerStrategy !== "none" && (
          <div className="param-row">
            <label title={t("param.maxLayersTitle")}>{t("param.maxLayers")}</label>
            <input
              type="number"
              min={2}
              max={16}
              value={layerParams.layers || 4}
              onChange={e => set("layers")(Number(e.target.value) || 4)}
            />
          </div>
        )}
        {(layerParams.layerStrategy === "complexity" || layerParams.layerStrategy === "hybrid") && (
          <Slider
            label={t("param.complexityThreshold")}
            title={t("param.complexityThresholdTitle")}
            value={layerParams.complexityThreshold ?? 70}
            min={10}
            max={90}
            step={5}
            onChange={set("complexityThreshold")}
          />
        )}
      </Group>

      <button
        className="primary process-btn"
        onClick={onProcess}
        disabled={!processPending || processBusy}
        title={t("layer.processHint")}
      >
        {processBusy ? t("processing") : t("layer.process")}
      </button>

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

      {layers.length === 0 ? (
        <div className="muted">
          {processed ? t("layer.noData") : t("layer.noDataUnprocessed")}
        </div>
      ) : (
        layers.map(layer => (
          <LayerCard
            key={layer.name}
            layer={layer}
            canvasSize={canvasSize}
            selected={selected.has(layer.name)}
            onToggle={() => toggle(layer.name)}
          />
        ))
      )}

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
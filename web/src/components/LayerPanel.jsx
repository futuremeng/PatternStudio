import React, { useState } from "react";
import JSZip from "jszip";
import { downloadSvg, downloadBlob } from "../api.js";
import { useI18n } from "../i18n/index.jsx";
import { layerToStandaloneSvg, layerToThumbnailSvg } from "../editorUtils.js";
import { SegmentedControl, Slider, Group } from "./ParameterPanel.jsx";

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
  layerParams, onLayerParamsChange, onProcess, onCancel, processBusy, processPending, processed,
  canvasSize,
  savedLayers = [], editingLayerId = null, onUseAsEditingObject, onRestoreFullResult,
}) {
  const { t } = useI18n();
  const [selected, setSelected] = useState(new Set());
  const [exporting, setExporting] = useState(false);
  // The exported layer whose "switch editing object" prompt is open (null = no
  // prompt). Switching replaces what the editing area shows and edits, so it
  // is always confirmed first.
  const [confirmLayer, setConfirmLayer] = useState(null);
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

  // Download one exported layer as its own .svg (they are no longer part of the
  // checkbox list above, so they keep their own export action).
  const exportOne = async (layer) => {
    await downloadSvg(layerToStandaloneSvg(layer.previewSvg, canvasSize), `${sanitizeFilename(layer.name)}.svg`);
  };

  // Clicking an exported layer's picture asks whether it should become the
  // object the editing area works on; clicking the one already in use switches
  // back to the full layered result.
  const pickEditingLayer = (layer) => {
    if (editingLayerId === layer.id) onRestoreFullResult?.();
    else setConfirmLayer(layer);
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

      {/* Manual processing. Auto layering re-fits every shape and can take a
          long time, so while it runs the row also offers an abort. */}
      <div className="process-row">
        <button
          className="primary process-btn"
          onClick={onProcess}
          disabled={!processPending || processBusy}
          title={t("layer.processHint")}
        >
          {processBusy ? t("processing") : t("layer.process")}
        </button>
        {processBusy && onCancel && (
          <button
            className="cancel-btn"
            onClick={onCancel}
            title={t("layer.cancelProcessingHint")}
          >
            {t("layer.cancelProcessing")}
          </button>
        )}
      </div>

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

      {/* Exported layers live in their own list at the bottom: each picture is
          a candidate editing object for the area on the left. */}
      {savedLayers.length > 0 && (
        <div className="saved-layers">
          <div className="saved-layers-header">
            <h3>{t("layer.exportedTitle", { n: savedLayers.length })}</h3>
            {editingLayerId != null && (
              <button
                onClick={onRestoreFullResult}
                style={{ fontSize: "0.75rem", padding: "0.2rem 0.5rem" }}
                title={t("layer.restoreFullHint")}
              >
                {t("layer.restoreFull")}
              </button>
            )}
          </div>
          {savedLayers.map(layer => (
            <div
              key={layer.id}
              className={`layer-card saved-layer-card${editingLayerId === layer.id ? " editing" : ""}`}
              onClick={() => pickEditingLayer(layer)}
              title={editingLayerId === layer.id ? t("layer.editingHint") : t("layer.useAsEditingHint")}
            >
              <div className="mini-preview" dangerouslySetInnerHTML={{ __html: layerToThumbnailSvg(layer.previewSvg, canvasSize) }} />
              <div className="info">
                <div className="name">
                  {layer.name}
                  {editingLayerId === layer.id && (
                    <span className="editing-badge">{t("layer.editingBadge")}</span>
                  )}
                </div>
                <div className="meta">
                  <span className="color-dot" style={{ background: layer.dominantColor }} />
                  <span>{t("layer.shapes", { n: layer.shapeCount })}</span>
                </div>
              </div>
              <button
                className="saved-layer-export"
                onClick={e => { e.stopPropagation(); exportOne(layer); }}
                title={t("layer.exportOneHint")}
                aria-label={t("layer.exportOne")}
              >
                ↓
              </button>
            </div>
          ))}
        </div>
      )}

      {confirmLayer && (
        <div className="modal-overlay" onClick={() => setConfirmLayer(null)}>
          <div className="modal confirm-modal" onClick={e => e.stopPropagation()}>
            <div className="modal-header">
              <h2>{t("layer.exportedTitle", { n: savedLayers.length })}</h2>
              <button className="icon-btn" onClick={() => setConfirmLayer(null)} aria-label={t("close")}>×</button>
            </div>
            <div className="confirm-body">
              <div
                className="confirm-preview"
                dangerouslySetInnerHTML={{ __html: layerToThumbnailSvg(confirmLayer.previewSvg, canvasSize) }}
              />
              <p>{t("layer.useAsEditingConfirm", { name: confirmLayer.name })}</p>
            </div>
            <div className="modal-footer">
              <button onClick={() => setConfirmLayer(null)}>{t("settings.cancel")}</button>
              <button
                className="primary"
                onClick={() => { onUseAsEditingObject?.(confirmLayer); setConfirmLayer(null); }}
              >
                {t("layer.useAsEditingOk")}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
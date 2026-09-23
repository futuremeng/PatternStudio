import React, { useState } from "react";
import { useI18n } from "../i18n/index.jsx";

function SegmentedControl({ label, value, options, onChange, title }) {
  return (
    <div className="param-row">
      <label title={title}>{label}</label>
      <div className="seg-control">
        {options.map(opt => (
          <button
            key={opt.value}
            className={value === opt.value ? "active" : ""}
            onClick={() => onChange(opt.value)}
            title={opt.title}
          >
            {opt.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function Slider({ label, value, min, max, step, onChange, title }) {
  return (
    <div className="param-row">
      <label title={title}>{label}</label>
      <div className="slider-row">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={e => onChange(Number(e.target.value))}
        />
        <span className="val">{value}</span>
      </div>
    </div>
  );
}

/** Collapsible parameter group; body renders only when open. */
function Group({ title, defaultOpen = true, children }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="param-section">
      <button type="button" className="param-section-header" onClick={() => setOpen(o => !o)}>
        <span className="chevron">{open ? "▼" : "▶"}</span>
        {title}
      </button>
      {open && <div className="param-section-body">{children}</div>}
    </div>
  );
}

export default function ParameterPanel({
  params, onChange, onSmart, smartBusy,
  onPreprocess, onClearPreprocess, preprocessBusy, preprocessed,
  activeFamilyLabel,
}) {
  const { t } = useI18n();
  const set = (key) => (val) => onChange(prev => ({ ...prev, [key]: val }));

  const isSpline = params.mode === "spline";
  const maxColorsMode = params.maxColors == null || params.maxColors === 0 ? "auto" : "number";

  return (
    <div className="param-group">
      <button
        type="button"
        className={`preprocess-btn${preprocessed ? " on" : ""}${preprocessBusy ? " busy" : ""}`}
        onClick={() => {
          if (preprocessBusy) return;
          if (preprocessed) onClearPreprocess?.();
          else onPreprocess?.();
        }}
        disabled={preprocessBusy}
        title={t("param.preprocessTitle")}
      >
        {preprocessBusy
          ? t("param.preprocessBusy")
          : preprocessed
            ? `✓ ${t("param.preprocessOn")}`
            : t("param.preprocess")}
      </button>
      <button
        type="button"
        className={`smart-tune${smartBusy ? " busy" : ""}`}
        onClick={() => !smartBusy && onSmart && onSmart()}
        disabled={smartBusy}
        title={t("param.smartTitle", { family: activeFamilyLabel || t("param.currentPreset") })}
      >
        {smartBusy
          ? t("param.smartTuning")
          : activeFamilyLabel
            ? t("param.smartTuneFamily", { family: activeFamilyLabel })
            : t("param.smartTune")}
      </button>
      <Group title={t("param.groupClustering")}>
        <SegmentedControl
          label={t("param.clusteringMode")}
          value={params.clustering}
          options={[
            { value: "binary", label: "B/W" },
            { value: "color", label: t("param.color") },
            { value: "watershed", label: t("param.watershed") },
          ]}
          onChange={set("clustering")}
        />
        {params.clustering === "binary" && (
          <>
            <SegmentedControl
              label={t("param.thresholdMode")}
              value={params.binaryAdaptive ? "adaptive" : "fixed"}
              options={[
                { value: "fixed", label: t("param.fixed") },
                { value: "adaptive", label: t("param.adaptive") },
              ]}
              onChange={v => set("binaryAdaptive")(v === "adaptive")}
            />
            {!params.binaryAdaptive && (
              <Slider
                label={t("param.threshold")}
                value={params.binaryThreshold ?? 128}
                min={0}
                max={255}
                step={1}
                onChange={set("binaryThreshold")}
              />
            )}
            {params.binaryAdaptive && (
              <>
                <Slider
                  label={t("param.window")}
                  title={t("param.windowTitle")}
                  value={params.binaryAdaptiveWindow ?? 0}
                  min={0}
                  max={128}
                  step={1}
                  onChange={set("binaryAdaptiveWindow")}
                />
                <Slider
                  label={t("param.sensitivity")}
                  title={t("param.sensitivityTitle")}
                  value={params.binaryAdaptiveT ?? 15}
                  min={0}
                  max={100}
                  step={1}
                  onChange={set("binaryAdaptiveT")}
                />
              </>
            )}
          </>
        )}
        {params.clustering === "color" && (
          <>
            <Slider
              label={t("param.colorPrecision")}
              value={params.colorPrecision}
              min={1}
              max={8}
              step={1}
              onChange={set("colorPrecision")}
            />
            <Slider
              label={t("param.layerDifference")}
              title={t("param.layerDifferenceTitle")}
              value={params.layerDifference}
              min={0}
              max={128}
              step={1}
              onChange={set("layerDifference")}
            />
          </>
        )}
        {params.clustering === "watershed" && (
          <Slider
            label={t("param.detail")}
            value={params.watershedDetail ?? 128}
            min={0}
            max={255}
            step={1}
            onChange={set("watershedDetail")}
          />
        )}
      </Group>

      <Group title={t("param.groupCompositing")}>
        <SegmentedControl
          label={t("param.compositing")}
          value={params.hierarchical}
          options={[
            { value: "stacked", label: t("param.stacked") },
            { value: "cutout", label: t("param.cutout") },
          ]}
          onChange={set("hierarchical")}
        />
        <Slider
          label={t("param.filterSpeckle")}
          value={params.filterSpeckle}
          min={0}
          max={20}
          step={1}
          onChange={set("filterSpeckle")}
        />
      </Group>

      <Group title={t("param.groupCurveFitting")}>
        <SegmentedControl
          label={t("param.mode")}
          value={params.mode}
          options={[
            { value: "pixel", label: t("param.pixel") },
            { value: "polygon", label: t("param.polygon") },
            { value: "spline", label: t("param.spline") },
          ]}
          onChange={set("mode")}
        />

        {isSpline && (
          <>
            <Slider
              label={t("param.simplify")}
              title={t("param.simplifyTitle")}
              value={params.simplify}
              min={0}
              max={0.25}
              step={0.01}
              onChange={set("simplify")}
            />
            <Slider
              label={t("param.pathPrecision")}
              value={params.pathPrecision}
              min={0}
              max={6}
              step={1}
              onChange={set("pathPrecision")}
            />
            <Slider
              label={t("param.cornerThreshold")}
              title={t("param.cornerThresholdTitle")}
              value={params.cornerThreshold ?? 60}
              min={0}
              max={180}
              step={5}
              onChange={set("cornerThreshold")}
            />
            <Slider
              label={t("param.lengthThreshold")}
              title={t("param.lengthThresholdTitle")}
              value={params.lengthThreshold ?? 4}
              min={0}
              max={20}
              step={0.5}
              onChange={set("lengthThreshold")}
            />
          </>
        )}
      </Group>

      <Group title={t("param.groupColorFitting")}>
        <SegmentedControl
          label={t("param.maxColors")}
          value={maxColorsMode}
          options={[
            { value: "auto", label: t("param.auto") },
            { value: "number", label: t("param.number") },
          ]}
          onChange={v => set("maxColors")(v === "auto" ? null : (params.maxColors || 8))}
        />
        {maxColorsMode === "number" && (
          <div className="param-row">
            <label>{t("param.colorCount")}</label>
            <input
              type="number"
              min={1}
              max={256}
              value={params.maxColors}
              onChange={e => set("maxColors")(Number(e.target.value) || null)}
            />
          </div>
        )}
        <div className="param-row">
          <label>{t("param.palette")}</label>
          <input
            type="text"
            placeholder="#ff0000,#00ff00"
            value={params.palette || ""}
            onChange={e => set("palette")(e.target.value)}
          />
        </div>
      </Group>

      <Group title={t("param.groupLayerStrategy")} defaultOpen={false}>
        <SegmentedControl
          label={t("param.layerStrategy")}
          title={t("param.layerStrategyTitle")}
          value={params.layerStrategy ?? "none"}
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
        {params.layerStrategy !== "none" && (
          <div className="param-row">
            <label title={t("param.maxLayersTitle")}>{t("param.maxLayers")}</label>
            <input
              type="number"
              min={2}
              max={16}
              value={params.layers || 4}
              onChange={e => set("layers")(Number(e.target.value) || 4)}
            />
          </div>
        )}
        {(params.layerStrategy === "complexity" || params.layerStrategy === "hybrid") && (
          <Slider
            label={t("param.complexityThreshold")}
            title={t("param.complexityThresholdTitle")}
            value={params.complexityThreshold ?? 70}
            min={10}
            max={90}
            step={5}
            onChange={set("complexityThreshold")}
          />
        )}
        <SegmentedControl
          label={t("param.optimize")}
          title={t("param.optimizeTitle")}
          value={String(params.optimize ?? 1)}
          options={[
            { value: "0", label: t("param.off"), title: t("param.offTitle") },
            { value: "1", label: t("param.standard"), title: t("param.standardTitle") },
            { value: "2", label: t("param.deep"), title: t("param.deepTitle") },
          ]}
          onChange={v => set("optimize")(Number(v))}
        />
      </Group>

      <Group title={t("param.groupOutput")}>
        <SegmentedControl
          label={t("param.outputStyle")}
          title={t("param.outputStyleTitle")}
          value={params.lineArt ? "line" : "fill"}
          options={[
            { value: "fill", label: t("param.fill"), title: t("param.fillTitle") },
            { value: "line", label: t("param.lineArt"), title: t("param.lineArtTitle") },
          ]}
          onChange={v => set("lineArt")(v === "line")}
        />
      </Group>
    </div>
  );
}
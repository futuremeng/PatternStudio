import React from "react";

function SegmentedControl({ label, value, options, onChange }) {
  return (
    <div className="param-row">
      <label>{label}</label>
      <div className="seg-control">
        {options.map(opt => (
          <button
            key={opt.value}
            className={value === opt.value ? "active" : ""}
            onClick={() => onChange(opt.value)}
          >
            {opt.label}
          </button>
        ))}
      </div>
    </div>
  );
}

function Slider({ label, value, min, max, step, onChange }) {
  return (
    <div className="param-row">
      <label>{label}</label>
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

export default function ParameterPanel({ params, onChange }) {
  const set = (key) => (val) => onChange(prev => ({ ...prev, [key]: val }));

  return (
    <div className="param-group">
      <SegmentedControl
        label="Preset"
        value={params.preset}
        options={[
          { value: "poster", label: "海报" },
          { value: "bw", label: "黑白" },
          { value: "photo", label: "照片" },
        ]}
        onChange={set("preset")}
      />

      <SegmentedControl
        label="聚类模式"
        value={params.clustering}
        options={[
          { value: "color", label: "颜色" },
          { value: "binary", label: "二值" },
          { value: "watershed", label: "分水岭" },
        ]}
        onChange={set("clustering")}
      />

      <SegmentedControl
        label="组合方式"
        value={params.hierarchical}
        options={[
          { value: "stacked", label: "堆叠" },
          { value: "cutout", label: "切割" },
        ]}
        onChange={set("hierarchical")}
      />

      <SegmentedControl
        label="曲线拟合"
        value={params.mode}
        options={[
          { value: "pixel", label: "像素" },
          { value: "polygon", label: "多边形" },
          { value: "spline", label: "样条" },
        ]}
        onChange={set("mode")}
      />

      <Slider
        label="颜色精度 (1-8)"
        value={params.colorPrecision}
        min={1}
        max={8}
        step={1}
        onChange={set("colorPrecision")}
      />

      <Slider
        label="层差阈值 (1-64)"
        value={params.layerDifference}
        min={1}
        max={64}
        step={1}
        onChange={set("layerDifference")}
      />

      <Slider
        label="噪点过滤 (0-20)"
        value={params.filterSpeckle}
        min={0}
        max={20}
        step={1}
        onChange={set("filterSpeckle")}
      />

      <Slider
        label="简化程度 (0-5)"
        value={params.simplify}
        min={0}
        max={5}
        step={0.1}
        onChange={set("simplify")}
      />

      <Slider
        label="路径精度 (0-6)"
        value={params.pathPrecision}
        min={0}
        max={6}
        step={1}
        onChange={set("pathPrecision")}
      />

      <div className="param-row">
        <label>最大颜色数 (0 = 不限)</label>
        <input
          type="number"
          min={0}
          max={256}
          value={params.maxColors || 0}
          onChange={e => set("maxColors")(Number(e.target.value) || null)}
        />
      </div>

      <div className="param-row">
        <label>自动分层数 (0 = 单层)</label>
        <input
          type="number"
          min={0}
          max={16}
          value={params.layers}
          onChange={e => set("layers")(Number(e.target.value))}
        />
      </div>

      <div className="param-row">
        <label style={{ display: "flex", alignItems: "center", gap: "0.5rem" }}>
          <input
            type="checkbox"
            checked={params.lineArt}
            onChange={e => set("lineArt")(e.target.checked)}
          />
          线稿模式 (stroke 代替 fill)
        </label>
      </div>
    </div>
  );
}
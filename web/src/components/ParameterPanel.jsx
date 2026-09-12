import React, { useState } from "react";

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

export default function ParameterPanel({ params, onChange }) {
  const set = (key) => (val) => onChange(prev => ({ ...prev, [key]: val }));

  const isSpline = params.mode === "spline";
  const maxColorsMode = params.maxColors == null || params.maxColors === 0 ? "auto" : "number";

  return (
    <div className="param-group">
      <Group title="Clustering">
        <SegmentedControl
          label="聚类模式"
          value={params.clustering}
          options={[
            { value: "binary", label: "B/W" },
            { value: "color", label: "颜色" },
            { value: "watershed", label: "分水岭" },
          ]}
          onChange={set("clustering")}
        />
        {params.clustering === "binary" && (
          <>
            <SegmentedControl
              label="阈值模式"
              value={params.binaryAdaptive ? "adaptive" : "fixed"}
              options={[
                { value: "fixed", label: "固定" },
                { value: "adaptive", label: "自适应" },
              ]}
              onChange={v => set("binaryAdaptive")(v === "adaptive")}
            />
            {!params.binaryAdaptive && (
              <Slider
                label="阈值 (0-255)"
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
                  label="Window (0-128)"
                  title="自适应阈值窗口边长（像素）；0 = 自动（约为短边的 1/8）"
                  value={params.binaryAdaptiveWindow ?? 0}
                  min={0}
                  max={128}
                  step={1}
                  onChange={set("binaryAdaptiveWindow")}
                />
                <Slider
                  label="Sensitivity (0-100)"
                  title="自适应灵敏度：低于局部均值 t% 的像素视为前景（默认 15）"
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
              label="颜色精度 (1-8)"
              value={params.colorPrecision}
              min={1}
              max={8}
              step={1}
              onChange={set("colorPrecision")}
            />
            <Slider
              label="渐变步长 (0-128)"
              title="相邻渐变层之间的颜色差（越大层数越少）"
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
            label="细节 (0-255)"
            value={params.watershedDetail ?? 128}
            min={0}
            max={255}
            step={1}
            onChange={set("watershedDetail")}
          />
        )}
      </Group>

      <Group title="Compositing">
        <SegmentedControl
          label="组合方式"
          value={params.hierarchical}
          options={[
            { value: "stacked", label: "堆叠" },
            { value: "cutout", label: "切割" },
          ]}
          onChange={set("hierarchical")}
        />
        <Slider
          label="噪点过滤 (0-20)"
          value={params.filterSpeckle}
          min={0}
          max={20}
          step={1}
          onChange={set("filterSpeckle")}
        />
      </Group>

      <Group title="Curve Fitting">
        <SegmentedControl
          label="模式"
          value={params.mode}
          options={[
            { value: "pixel", label: "像素" },
            { value: "polygon", label: "多边形" },
            { value: "spline", label: "样条" },
          ]}
          onChange={set("mode")}
        />

        {isSpline && (
          <>
            <Slider
              label="简化程度 (0-0.25)"
              title="曲线简化容差（像素）：在容差内用更少的曲线重拟合平滑段，0 = 不简化"
              value={params.simplify}
              min={0}
              max={0.25}
              step={0.01}
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
            <Slider
              label="角点阈值 (0-180°)"
              title="角度小于此值的拐点会被平滑掉（值越大越平滑）"
              value={params.cornerThreshold ?? 60}
              min={0}
              max={180}
              step={5}
              onChange={set("cornerThreshold")}
            />
            <Slider
              label="线段长度 (0-20)"
              title="短于此长度的线段会被过滤掉（值越大越简洁）"
              value={params.lengthThreshold ?? 4}
              min={0}
              max={20}
              step={0.5}
              onChange={set("lengthThreshold")}
            />
          </>
        )}
      </Group>

      <Group title="Color Fitting">
        <SegmentedControl
          label="最大颜色数"
          value={maxColorsMode}
          options={[
            { value: "auto", label: "自动" },
            { value: "number", label: "数字" },
          ]}
          onChange={v => set("maxColors")(v === "auto" ? null : (params.maxColors || 8))}
        />
        {maxColorsMode === "number" && (
          <div className="param-row">
            <label>颜色数 (1-256)</label>
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
          <label>固定色板 (空 = 不启用)</label>
          <input
            type="text"
            placeholder="#ff0000,#00ff00"
            value={params.palette || ""}
            onChange={e => set("palette")(e.target.value)}
          />
        </div>
      </Group>

      <Group title="Layer Strategy" defaultOpen={false}>
        <SegmentedControl
          label="分层策略"
          title="控制如何把矢量形状分配到不同图层"
          value={params.layerStrategy ?? "none"}
          options={[
            { value: "none", label: "单层", title: "所有形状放在同一个图层" },
            { value: "auto", label: "自动", title: "根据当前参数组合自动选择分层策略，并按 粗/标准/细 三级参数剖面处理各层" },
            { value: "color", label: "颜色", title: "相同颜色的形状归到同一图层" },
            { value: "complexity", label: "复杂度", title: "简单形状作为底图，复杂形状单独成层" },
            { value: "hybrid", label: "混合", title: "先按颜色分组，再在每组内按复杂度拆分出细节层" },
          ]}
          onChange={set("layerStrategy")}
        />
        {params.layerStrategy !== "none" && (
          <div className="param-row">
            <label title="最多生成的图层数量">最大层数 (2-16)</label>
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
            label="复杂度阈值 (10-90%)"
            title="路径命令数超过此百分比的形状会被拆到细节层"
            value={params.complexityThreshold ?? 70}
            min={10}
            max={90}
            step={5}
            onChange={set("complexityThreshold")}
          />
        )}
        <SegmentedControl
          label="优化级别"
          title="路径优化强度：关 / 标准 / 深度"
          value={String(params.optimize ?? 1)}
          options={[
            { value: "0", label: "关", title: "不做路径优化" },
            { value: "1", label: "标准", title: "标准路径优化" },
            { value: "2", label: "深度", title: "深度路径优化（更慢，路径更简洁）" },
          ]}
          onChange={v => set("optimize")(Number(v))}
        />
      </Group>

      <Group title="输出">
        <SegmentedControl
          label="输出样式"
          title="填充 (fill) 或线稿 (stroke)"
          value={params.lineArt ? "line" : "fill"}
          options={[
            { value: "fill", label: "填充", title: "彩色填充形状" },
            { value: "line", label: "线稿", title: "用描边 (stroke) 代替填充 (fill) 输出线稿" },
          ]}
          onChange={v => set("lineArt")(v === "line")}
        />
      </Group>
    </div>
  );
}
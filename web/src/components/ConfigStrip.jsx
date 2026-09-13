import React from "react";

// Thumbnail kinds, visually distinguished on the strip:
// - preset: system presets (isDefault)
// - saved:  user-saved configs (persisted, `saved: true`)
// - temp:   temporary in-session snapshots (not persisted)
const KIND_META = {
  preset: "预设",
  saved: "已保存",
  temp: "临时",
};

/**
 * Bottom strip of parameter-config combinations:
 * default presets (kept) + user-generated snapshots (deletable).
 * Clicking a card loads its full param set (auto-trace picks it up).
 */
export default function ConfigStrip({
  configs,
  activeConfigId,
  onConfigSelect,
  onConfigSave,
  onConfigDelete,
}) {
  return (
    <div className="mode-strip">
      {configs.map(config => {
        const isActive = activeConfigId === config.id;
        const kind = config.isDefault ? "preset" : config.saved ? "saved" : "temp";
        return (
          <div
            key={config.id}
            className={`mode-thumb kind-${kind}${isActive ? " active" : ""}`}
            onClick={() => onConfigSelect(config)}
            title={`${config.label}（${KIND_META[kind]}）`}
          >
            {config.result?.svg ? (
              <div className="svg-mini" dangerouslySetInnerHTML={{ __html: config.result.svg }} />
            ) : (
              <div className="svg-mini placeholder-mini">{config.label}</div>
            )}
            <span className={`thumb-tag thumb-tag-${kind}`}>{KIND_META[kind]}</span>
            <span className="mode-label">{config.label}</span>
            {!config.isDefault && (
              <>
                <button
                  className={`config-save${config.saved ? " saved" : ""}`}
                  title={config.saved ? "已保存，点击取消保存" : "保存此配置"}
                  onClick={e => { e.stopPropagation(); onConfigSave(config); }}
                >
                  {config.saved ? "✓" : "✚"}
                </button>
                <button
                  className="config-delete"
                  title="删除此配置"
                  onClick={e => { e.stopPropagation(); onConfigDelete(config.id); }}
                >
                  ×
                </button>
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}
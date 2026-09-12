import React from "react";

/**
 * Bottom strip of parameter-config combinations:
 * default presets (kept) + user-generated snapshots (deletable).
 * Clicking a card loads its full param set (auto-trace picks it up).
 */
export default function ConfigStrip({ configs, activeConfigId, onConfigSelect, onConfigSave, onConfigDelete }) {
  return (
    <div className="mode-strip">
      {configs.map(config => {
        const isActive = activeConfigId === config.id;
        return (
          <div
            key={config.id}
            className={`mode-thumb ${isActive ? "active" : ""}`}
            onClick={() => onConfigSelect(config)}
            title={config.label}
          >
            {config.result?.svg ? (
              <div className="svg-mini" dangerouslySetInnerHTML={{ __html: config.result.svg }} />
            ) : (
              <div className="svg-mini placeholder-mini">{config.label}</div>
            )}
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
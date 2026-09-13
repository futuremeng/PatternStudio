import React, { useState, useRef, useCallback } from "react";

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
  onConfigRename,
}) {
  // Inline rename state for user configs (double-click the label).
  const [editingId, setEditingId] = useState(null);
  const [draft, setDraft] = useState("");
  const editingIdRef = useRef(null);
  const setEditing = (id) => { editingIdRef.current = id; setEditingId(id); };

  const startEdit = useCallback((e, config) => {
    e.stopPropagation();
    setDraft(config.label);
    setEditing(config.id);
  }, []);

  /** Commit the rename (Enter / blur). Empty names revert to the old label. */
  const commitEdit = useCallback((config) => {
    if (editingIdRef.current !== config.id) return;
    setEditing(null);
    const label = draft.trim();
    if (label && label !== config.label) onConfigRename(config.id, label);
  }, [draft, onConfigRename]);

  const cancelEdit = useCallback(() => {
    if (editingIdRef.current === null) return;
    setEditing(null);
  }, []);

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
            {editingId === config.id ? (
              <input
                className="mode-label-input"
                value={draft}
                autoFocus
                maxLength={20}
                onClick={e => e.stopPropagation()}
                onDoubleClick={e => e.stopPropagation()}
                onChange={e => setDraft(e.target.value)}
                onBlur={() => commitEdit(config)}
                onKeyDown={e => {
                  e.stopPropagation();
                  if (e.key === "Enter") commitEdit(config);
                  else if (e.key === "Escape") cancelEdit();
                }}
              />
            ) : (
              <span
                className="mode-label"
                onDoubleClick={config.isDefault ? undefined : e => startEdit(e, config)}
                title={config.isDefault ? undefined : "双击改名"}
              >
                {config.label}
              </span>
            )}
            {config.isDefault ? (
              <span className="config-kind-icon" title="系统预设">⚙</span>
            ) : (
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
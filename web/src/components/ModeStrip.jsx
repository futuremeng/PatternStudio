import React from "react";
import { MODE_PRESETS } from "../api.js";

export default function ModeStrip({ activeMode, onModeSelect, modeResults, imageUrl }) {
  return (
    <div className="mode-strip">
      {Object.entries(MODE_PRESETS).map(([key, preset]) => {
        const result = modeResults[key];
        const isActive = activeMode === key;
        return (
          <div
            key={key}
            className={`mode-thumb ${isActive ? "active" : ""}`}
            onClick={() => onModeSelect(key)}
            title={preset.label}
          >
            {key === "original" && imageUrl ? (
              <img src={imageUrl} alt={preset.label} />
            ) : result?.svg ? (
              <div className="svg-mini" dangerouslySetInnerHTML={{ __html: result.svg }} />
            ) : (
              <div className="svg-mini" style={{ display: "flex", alignItems: "center", justifyContent: "center", fontSize: "0.7rem", color: "var(--text2)" }}>
                {preset.label}
              </div>
            )}
            <span className="mode-label">{preset.label}</span>
          </div>
        );
      })}
    </div>
  );
}
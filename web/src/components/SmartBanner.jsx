import React from "react";
import { MODE_PRESETS } from "../api.js";
import { stableStringify } from "../traceCache.js";

/**
 * Floating banner summarizing the last Smart Preset decision: the winning
 * candidate, a human-readable reason, the preset baseline's metrics, and a
 * one-click fallback to the preset result. Renders nothing until a decision
 * with params has arrived.
 */
export default function SmartBanner({ smartInfo, t, presetLabel, onApplyPreset, onClose }) {
  if (!smartInfo?.params) return null;

  // The actually-applied smart winner (may be the preset baseline when no
  // variation clearly beat it), and whether it *is* the preset.
  const smartWinner = smartInfo.candidates?.find(
    c => stableStringify(c.params) === stableStringify(smartInfo.params)
  );
  const smartIsPreset = !!(
    smartInfo.preset &&
    stableStringify(smartInfo.preset.params) === stableStringify(smartInfo.params)
  );

  // Candidate labels arrive as i18n keys: preset family ids (translated via
  // presetLabel) or "photo" / "flat" / "presetDefault" (candidate.* keys).
  const candidateLabel = (label) =>
    MODE_PRESETS[label] ? presetLabel(label) : t(`candidate.${label}`);

  // Human-readable smart decision, translated from the backend's reasonKey
  // ("scoped" = search was restricted to one preset family, incl. the preset
  // baseline). Legacy results carry a pre-built `reason` string instead.
  const smartReason = smartInfo.reasonKey
    ? t(smartInfo.preset ? "smart.reasonScoped" : "smart.reasonAuto", {
        colors: smartInfo.stats?.effectiveColors ?? 0,
        colorfulness: Math.round(smartInfo.stats?.colorfulness ?? 0),
        bimodality: (smartInfo.stats?.bimodality ?? 0).toFixed(2),
        texture: Math.round((smartInfo.stats?.texture ?? 0) * 100),
        family: presetLabel(smartInfo.family),
        n: smartInfo.candidates?.length ?? 0,
        winner: candidateLabel(smartWinner?.label || smartInfo.family),
        fidelity: (smartWinner?.fidelity ?? 0).toFixed(2),
        complexity: (smartWinner?.complexity ?? 0).toFixed(2),
        shapes: smartWinner?.shapeCount ?? 0,
      })
    : smartInfo.reason;

  return (
    <div className="smart-banner">
      <span className="smart-banner-icon">✨</span>
      <span className="smart-banner-text">
        {t("editor.smartLabel")}<strong>{candidateLabel(smartWinner?.label || smartInfo.family)}</strong>
        {smartReason ? ` — ${smartReason}` : ""}
        {smartInfo.preprocessed && (
          <span title={t("smart.preprocessedTitle")}> · {t("smart.preprocessed")}</span>
        )}
      </span>
      {smartInfo.preset && (
        <span
          className="smart-banner-compare"
          title={t("editor.presetMetricsTitle")}
        >
          {t("editor.presetMetrics", {
            f: smartInfo.preset.fidelity.toFixed(2),
            c: smartInfo.preset.complexity.toFixed(2),
          })}
        </span>
      )}
      {smartInfo.preset && !smartIsPreset && (
        <button className="smart-banner-preset" onClick={() => onApplyPreset(smartInfo.preset.params)}>
          {t("editor.usePresetResult")}
        </button>
      )}
      <button className="smart-banner-close" onClick={onClose} aria-label={t("close")}>×</button>
    </div>
  );
}

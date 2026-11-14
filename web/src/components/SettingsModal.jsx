import React, { useEffect, useRef, useState } from "react";
import { getSettings, saveSettings } from "../api.js";
import { useI18n } from "../i18n/index.jsx";
import { trapTabKey } from "../focusTrap.js";

/** Per-provider defaults, mirrored from the backend (redraw.rs) — used as
 *  input placeholders so the user can see what an empty field means. */
const PROVIDER_DEFAULTS = {
  dashscope: {
    apiBase: "https://dashscope.aliyuncs.com/api/v1",
    model: "qwen-image-2.0",
  },
  openai: {
    apiBase: "http://127.0.0.1:8080/v1",
    model: "qwen-image-edit",
  },
};

/**
 * Normalize an error thrown by the API layer into a display string. Tauri
 * command rejections are plain strings (not Error objects), so `e.message`
 * would be undefined there; the web path throws real Error objects.
 */
function errMsg(e) {
  return typeof e === "string" ? e : (e?.message || String(e));
}

/**
 * Settings dialog, mounted once in App and driven by settingsStore — opened
 * from the gear button in the app header, the ⌘, shortcut or the Settings…
 * menu item. Currently holds the Qwen-Image (redraw) API configuration:
 * protocol, API key, base URL and model. The API key is only ever shown
 * masked; leaving the field empty on save keeps the current key.
 */
export default function SettingsModal({ open, onClose }) {
  const { t } = useI18n();
  const modalRef = useRef(null);
  const [provider, setProvider] = useState("dashscope");
  const [apiKey, setApiKey] = useState("");
  const [apiBase, setApiBase] = useState("");
  const [model, setModel] = useState("");
  const [masked, setMasked] = useState("");
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState(null);

  useEffect(() => {
    if (!open) return;
    setSaved(false);
    setError(null);
    getSettings()
      .then((s) => {
        const q = s.qwen || {};
        setProvider(q.provider || "dashscope");
        setApiBase(q.apiBase || "");
        setModel(q.model || "");
        setMasked(q.apiKeyMasked || "");
      })
      .catch((e) => setError(errMsg(e)));
  }, [open]);

  // Close on Escape (when not saving); move focus into the dialog on open.
  useEffect(() => {
    if (!open) return;
    modalRef.current?.focus();
    const onKey = (e) => {
      if (e.key === "Escape" && !saving) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, saving, onClose]);

  if (!open) return null;

  const defaults = PROVIDER_DEFAULTS[provider] || PROVIDER_DEFAULTS.dashscope;

  const save = async () => {
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      const out = await saveSettings({
        provider,
        apiKey: apiKey.trim(),
        apiBase: apiBase.trim(),
        model: model.trim(),
      });
      const q = out.qwen || {};
      setMasked(q.apiKeyMasked || "");
      setApiKey("");
      setSaved(true);
    } catch (e) {
      setError(errMsg(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-overlay settings-modal-overlay">
      <div
        ref={modalRef}
        className="modal settings-modal"
        role="dialog"
        aria-modal="true"
        aria-label={t("settings.title")}
        tabIndex={-1}
        onKeyDown={(e) => trapTabKey(modalRef.current, e)}
      >
        <div className="modal-header">
          <h2>{t("settings.title")}</h2>
          <button
            className="icon-btn"
            onClick={onClose}
            disabled={saving}
            aria-label={t("close")}
          >
            ×
          </button>
        </div>
        <div className="settings-body">
          <h3 className="settings-section-title">{t("settings.qwenTitle")}</h3>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-provider">
              {t("settings.provider")}
            </label>
            <select
              id="settings-provider"
              className="settings-input"
              value={provider}
              onChange={(e) => setProvider(e.target.value)}
              disabled={saving}
            >
              <option value="dashscope">{t("settings.provider.dashscope")}</option>
              <option value="openai">{t("settings.provider.openai")}</option>
            </select>
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-api-key">
              {t("settings.apiKey")}
            </label>
            <input
              id="settings-api-key"
              className="settings-input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={
                masked
                  ? `${masked} (${t("settings.apiKeyKeep")})`
                  : t("settings.apiKeyPlaceholder")
              }
              disabled={saving}
            />
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-api-base">
              {t("settings.apiBase")}
            </label>
            <input
              id="settings-api-base"
              className="settings-input"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={apiBase}
              onChange={(e) => setApiBase(e.target.value)}
              placeholder={defaults.apiBase}
              disabled={saving}
            />
          </div>
          <div className="settings-row">
            <label className="settings-label" htmlFor="settings-model">
              {t("settings.model")}
            </label>
            <input
              id="settings-model"
              className="settings-input"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={model}
              onChange={(e) => setModel(e.target.value)}
              placeholder={defaults.model}
              disabled={saving}
            />
          </div>
          <p className="settings-hint">{t("settings.storedIn")}</p>
          {error && <p className="settings-error">{error}</p>}
        </div>
        <div className="modal-footer settings-modal-footer">
          {saved && <span className="settings-saved">✓ {t("settings.saved")}</span>}
          <button onClick={onClose} disabled={saving}>
            {t("settings.cancel")}
          </button>
          <button className="primary" onClick={save} disabled={saving}>
            {saving ? t("settings.saving") : t("settings.save")}
          </button>
        </div>
      </div>
    </div>
  );
}


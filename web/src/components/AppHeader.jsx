import React, { useState, useSyncExternalStore } from "react";
import { useI18n } from "../i18n/index.jsx";
import { getEffectiveTheme, subscribeTheme, toggleTheme } from "../theme.js";
import SettingsModal from "./SettingsModal.jsx";
import appIcon from "../assets/icon.png";

/**
 * Unified app header: brand + optional back button + page title/meta + actions.
 * Used by every page so the top bar looks the same everywhere.
 * The language switcher and settings dialog live here so they are available
 * on every page.
 */
export default function AppHeader({ title, meta, onBack, children }) {
  const { locale, setLocale, t, supported } = useI18n();
  const theme = useSyncExternalStore(subscribeTheme, getEffectiveTheme);
  const [settingsOpen, setSettingsOpen] = useState(false);
  return (
    <header className="app-header">
      <img className="brand" src={appIcon} alt="PatternStudio" />
      <span className="header-sep" aria-hidden="true" />
      {onBack && (
        <button className="back-btn" onClick={onBack}>{t("back")}</button>
      )}
      <h2 className="page-title" title={title}>{title}</h2>
      {meta && <span className="page-meta">{meta}</span>}
      <div className="header-actions">
        {children}
        <button
          className="theme-toggle"
          onClick={() => setSettingsOpen(true)}
          title={t("settings.title")}
          aria-label={t("settings.title")}
        >
          ⚙️
        </button>
        <button
          className="theme-toggle"
          onClick={toggleTheme}
          title={t("theme")}
          aria-label={t("theme")}
        >
          {theme === "dark" ? "☀️" : "🌙"}
        </button>
        <select
          className="lang-select"
          value={locale}
          onChange={e => setLocale(e.target.value)}
          title={t("language")}
          aria-label={t("language")}
        >
          {supported.map(l => (
            <option key={l.code} value={l.code}>{l.name}</option>
          ))}
        </select>
      </div>
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </header>
  );
}

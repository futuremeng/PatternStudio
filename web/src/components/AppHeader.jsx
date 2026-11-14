import React, { useSyncExternalStore } from "react";
import { useI18n } from "../i18n/index.jsx";
import { getEffectiveTheme, subscribeTheme, toggleTheme } from "../theme.js";
import { openSettings } from "../settingsStore.js";
import { IconGear, IconSun, IconMoon } from "./icons.jsx";
import appIcon from "../assets/icon.png";

/**
 * Unified app header: brand + optional back button + page title/meta + actions.
 * Used by every page so the top bar looks the same everywhere.
 * The language switcher lives here so it is available on every page; the
 * settings dialog is mounted globally (App) and driven by settingsStore so
 * it is also reachable via ⌘, and the Settings… menu item.
 */
export default function AppHeader({ title, meta, onBack, children }) {
  const { locale, setLocale, t, supported } = useI18n();
  const theme = useSyncExternalStore(subscribeTheme, getEffectiveTheme);
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
          onClick={() => openSettings()}
          title={t("settings.title")}
          aria-label={t("settings.title")}
        >
          <IconGear />
        </button>
        <button
          className="theme-toggle"
          onClick={toggleTheme}
          title={t("theme")}
          aria-label={t("theme")}
        >
          {theme === "dark" ? <IconSun /> : <IconMoon />}
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
    </header>
  );
}

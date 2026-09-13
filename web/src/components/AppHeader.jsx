import React from "react";
import { useI18n } from "../i18n/index.jsx";

/**
 * Unified app header: brand + optional back button + page title/meta + actions.
 * Used by every page so the top bar looks the same everywhere.
 * The language switcher lives here so it is available on every page.
 */
export default function AppHeader({ title, meta, onBack, children }) {
  const { locale, setLocale, t, supported } = useI18n();
  return (
    <header className="app-header">
      <span className="brand">PatternStudio</span>
      <span className="header-sep" aria-hidden="true" />
      {onBack && (
        <button className="back-btn" onClick={onBack}>{t("back")}</button>
      )}
      <h2 className="page-title" title={title}>{title}</h2>
      {meta && <span className="page-meta">{meta}</span>}
      <div className="header-actions">
        {children}
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

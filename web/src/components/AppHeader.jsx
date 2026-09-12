import React from "react";

/**
 * Unified app header: brand + optional back button + page title/meta + actions.
 * Used by every page so the top bar looks the same everywhere.
 */
export default function AppHeader({ title, meta, onBack, children }) {
  return (
    <header className="app-header">
      <span className="brand">PatternStudio</span>
      <span className="header-sep" aria-hidden="true" />
      {onBack && (
        <button className="back-btn" onClick={onBack}>← 返回</button>
      )}
      <h2 className="page-title" title={title}>{title}</h2>
      {meta && <span className="page-meta">{meta}</span>}
      {children && <div className="header-actions">{children}</div>}
    </header>
  );
}
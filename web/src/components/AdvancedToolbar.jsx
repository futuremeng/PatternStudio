import React from "react";
import { useI18n } from "../i18n/index.jsx";

// Inline SVG icons (16px)
const IconSelect = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
    <path d="M4 4l0 14 4.5-4.5 3 6.5 2.5-1.2-3-6.3 6 0z" />
  </svg>
);
const IconPath = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <circle cx="5" cy="19" r="2" /><circle cx="19" cy="5" r="2" />
    <path d="M7 19c0-6 4-12 10-12" />
  </svg>
);
const IconGroup = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <rect x="3" y="3" width="12" height="12" rx="2" />
    <rect x="9" y="9" width="12" height="12" rx="2" />
  </svg>
);
const IconUngroup = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <rect x="2" y="2" width="12" height="12" rx="2" />
    <rect x="10" y="10" width="12" height="12" rx="2" strokeDasharray="3 2" />
  </svg>
);
const IconMoveTop = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 19V5M5 12l7-7 7 7" />
  </svg>
);
const IconMoveBottom = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 5v14M19 12l-7 7-7-7" />
  </svg>
);
const IconMoveUp = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 18V6M6 12l6-6 6 6" />
  </svg>
);
const IconMoveDown = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 6v12M18 12l-6 6-6-6" />
  </svg>
);
const IconDelete = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6" />
  </svg>
);
const IconUndo = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M3 10h10a5 5 0 015 5v0a5 5 0 01-5 5H8" />
    <path d="M3 10l4-4M3 10l4 4" />
  </svg>
);
const IconRedo = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M21 10H11a5 5 0 00-5 5v0a5 5 0 005 5h5" />
    <path d="M21 10l-4-4M21 10l-4 4" />
  </svg>
);
const IconSaveLayer = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 3L3 8l9 5 9-5-9-5z" />
    <path d="M3 12.5l9 5 9-5" />
    <path d="M3 17l9 5 9-5" />
  </svg>
);
const IconCheck = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
    <path d="M20 6L9 17l-5-5" />
  </svg>
);
const IconX = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M18 6L6 18M6 6l12 12" />
  </svg>
);

/**
 * Toolbar for the advanced SVG editor.
 */
export default function AdvancedToolbar({
  mode, onModeChange,
  canUndo, canRedo, onUndo, onRedo,
  onGroup, onUngroup,
  onMoveTop, onMoveBottom, onMoveUp, onMoveDown,
  onDelete, onSaveAsLayer, onComplete, onCancel,
}) {
  const { t } = useI18n();

  return (
    <div className="advanced-toolbar">
      <div className="toolbar-group">
        <button className={mode === "select" ? "active" : ""} onClick={() => onModeChange("select")} title={t("adv.select")} aria-label={t("adv.select")}>
          <IconSelect />
        </button>
        <button className={mode === "path" ? "active" : ""} onClick={() => onModeChange("path")} title={t("adv.pathEdit")} aria-label={t("adv.pathEdit")}>
          <IconPath />
        </button>
      </div>
      <span className="toolbar-sep" />
      <div className="toolbar-group">
        <button onClick={onGroup} title={t("adv.group")} aria-label={t("adv.group")}>
          <IconGroup />
        </button>
        <button onClick={onUngroup} title={t("adv.ungroup")} aria-label={t("adv.ungroup")}>
          <IconUngroup />
        </button>
      </div>
      <span className="toolbar-sep" />
      <div className="toolbar-group">
        <button onClick={onMoveTop} title={t("adv.moveTop")} aria-label={t("adv.moveTop")}>
          <IconMoveTop />
        </button>
        <button onClick={onMoveUp} title={t("adv.moveUp")} aria-label={t("adv.moveUp")}>
          <IconMoveUp />
        </button>
        <button onClick={onMoveDown} title={t("adv.moveDown")} aria-label={t("adv.moveDown")}>
          <IconMoveDown />
        </button>
        <button onClick={onMoveBottom} title={t("adv.moveBottom")} aria-label={t("adv.moveBottom")}>
          <IconMoveBottom />
        </button>
      </div>
      <span className="toolbar-sep" />
      <div className="toolbar-group">
        <button className="danger" onClick={onDelete} title={t("adv.delete")} aria-label={t("adv.delete")}>
          <IconDelete />
        </button>
      </div>
      <span className="toolbar-sep" />
      <div className="toolbar-group">
        <button disabled={!canUndo} onClick={onUndo} title={t("adv.undo")} aria-label={t("adv.undo")}>
          <IconUndo />
        </button>
        <button disabled={!canRedo} onClick={onRedo} title={t("adv.redo")} aria-label={t("adv.redo")}>
          <IconRedo />
        </button>
      </div>
      <span className="toolbar-sep" />
      <div className="toolbar-group">
        <button onClick={onSaveAsLayer} title={t("adv.saveLayer")} aria-label={t("adv.saveLayer")}>
          <IconSaveLayer />
        </button>
      </div>
      <span className="toolbar-spacer" />
      <div className="toolbar-group">
        <button className="cancel" onClick={onCancel} title={t("adv.cancel")} aria-label={t("adv.cancel")}>
          <IconX />
        </button>
        <button className="complete" onClick={onComplete} title={t("adv.done")} aria-label={t("adv.done")}>
          <IconCheck />
        </button>
      </div>
    </div>
  );
}

import React, { useEffect, useRef } from "react";
import { useI18n } from "../i18n/index.jsx";
import { trapTabKey } from "../focusTrap.js";

/**
 * A small confirmation dialog for destructive or state-replacing actions.
 * Reuses the shared modal chrome; Escape and a click outside cancel.
 *
 * Props: title, children (body), confirmLabel, cancelLabel, onConfirm,
 *        onCancel, danger (render the confirm button as a destructive action).
 */
export default function ConfirmModal({
  title, children, confirmLabel, cancelLabel, onConfirm, onCancel, danger,
}) {
  const { t } = useI18n();
  const confirmRef = useRef(null);
  const modalRef = useRef(null);

  useEffect(() => {
    confirmRef.current?.focus();
    const onKey = (e) => { if (e.key === "Escape") onCancel?.(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="modal-overlay" onClick={() => onCancel?.()}>
      <div
        ref={modalRef}
        className="modal confirm-modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={e => e.stopPropagation()}
        onKeyDown={(e) => trapTabKey(modalRef.current, e)}
      >
        <div className="modal-header">
          <h2>{title}</h2>
          <button className="icon-btn" onClick={() => onCancel?.()} aria-label={t("close")}>×</button>
        </div>
        <div className="confirm-body">{children}</div>
        <div className="modal-footer">
          <button onClick={() => onCancel?.()}>{cancelLabel || t("settings.cancel")}</button>
          <button
            ref={confirmRef}
            className={danger ? "danger" : "primary"}
            onClick={() => onConfirm?.()}
          >
            {confirmLabel || t("confirm.ok")}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * Keep Tab / Shift+Tab cycling inside a dialog (Full Keyboard Access users
 * must not be able to tab into the inert background). Call from the dialog's
 * onKeyDown handler.
 */
const FOCUSABLE = [
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "a[href]",
  '[tabindex]:not([tabindex="-1"])',
].join(", ");

export function trapTabKey(container, e) {
  if (e.key !== "Tab") return;
  const els = Array.from(container.querySelectorAll(FOCUSABLE));
  if (!els.length) return;
  const first = els[0];
  const last = els[els.length - 1];
  const active = document.activeElement;
  if (e.shiftKey && (active === first || !container.contains(active))) {
    e.preventDefault();
    last.focus();
  } else if (!e.shiftKey && (active === last || !container.contains(active))) {
    e.preventDefault();
    first.focus();
  }
}

/**
 * Global settings-dialog visibility store (mirrors theme.js's pattern).
 * The modal itself is mounted once in App so it is reachable app-wide via
 * the ⌘, shortcut and the Settings… menu item; the header's gear button
 * just calls openSettings().
 */
let open = false;
const listeners = new Set();

function emit() {
  listeners.forEach(l => l());
}

export function getSettingsOpen() {
  return open;
}

export function openSettings() {
  if (open) return;
  open = true;
  emit();
}

export function closeSettings() {
  if (!open) return;
  open = false;
  emit();
}

/** Subscribe for useSyncExternalStore. */
export function subscribeSettings(cb) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

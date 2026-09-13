/**
 * Theme state: "system" (follow OS) | "light" | "dark".
 * The choice is persisted in localStorage and applied via
 * document.documentElement.dataset.theme, which the CSS in styles.css
 * maps onto the brand light/dark palettes.
 */
const STORAGE_KEY = "ps-theme";
const DARK_MEDIA = window.matchMedia("(prefers-color-scheme: dark)");

function readStored() {
  const v = localStorage.getItem(STORAGE_KEY);
  return v === "light" || v === "dark" ? v : "system";
}

let theme = readStored();
const listeners = new Set();

function apply() {
  if (theme === "system") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
}
apply();

function emit() {
  listeners.forEach(l => l());
}

// While in "system" mode, re-notify subscribers when the OS appearance flips
// so the toggle icon stays in sync.
DARK_MEDIA.addEventListener("change", () => {
  if (theme === "system") emit();
});

export function getStoredTheme() {
  return theme;
}

/** The theme actually in effect right now ("light" | "dark"). */
export function getEffectiveTheme() {
  if (theme !== "system") return theme;
  return DARK_MEDIA.matches ? "dark" : "light";
}

export function setTheme(next) {
  if (next !== "light" && next !== "dark" && next !== "system") return;
  theme = next;
  if (theme === "system") localStorage.removeItem(STORAGE_KEY);
  else localStorage.setItem(STORAGE_KEY, theme);
  apply();
  emit();
}

/** Flip between light and dark based on what is currently in effect. */
export function toggleTheme() {
  setTheme(getEffectiveTheme() === "dark" ? "light" : "dark");
}

/** Subscribe for useSyncExternalStore. */
export function subscribeTheme(cb) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

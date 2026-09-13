import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import zhCN from "./locales/zh-CN.js";
import zhTW from "./locales/zh-TW.js";
import en from "./locales/en.js";
import fr from "./locales/fr.js";
import ja from "./locales/ja.js";
import de from "./locales/de.js";
import it from "./locales/it.js";

/** Locales offered in the language switcher (display name in the locale itself). */
export const SUPPORTED_LOCALES = [
  { code: "zh-CN", name: "简体中文" },
  { code: "zh-TW", name: "繁體中文" },
  { code: "en", name: "English" },
  { code: "fr", name: "Français" },
  { code: "ja", name: "日本語" },
  { code: "de", name: "Deutsch" },
  { code: "it", name: "Italiano" },
];

const MESSAGES = { "zh-CN": zhCN, "zh-TW": zhTW, en, fr, ja, de, it };
const FALLBACK = "en";
const LOCALE_KEY = "patternstudio.locale";

/**
 * Resolve the initial locale:
 * 1. the user's explicit choice (persisted in localStorage),
 * 2. the OS/browser language (first supported match in navigator.languages),
 * 3. English.
 */
function detectLocale() {
  try {
    const saved = localStorage.getItem(LOCALE_KEY);
    if (saved && MESSAGES[saved]) return saved;
  } catch { /* storage unavailable */ }
  const candidates = typeof navigator !== "undefined"
    ? [...(navigator.languages || []), navigator.language || ""]
    : [];
  for (const raw of candidates) {
    if (!raw) continue;
    const [base, region] = raw.toLowerCase().split("-");
    if (base === "zh") {
      // Traditional variants map to zh-TW; everything else (incl. bare "zh") to zh-CN.
      return region === "tw" || region === "hk" || region === "mo" ? "zh-TW" : "zh-CN";
    }
    if (MESSAGES[base]) return base;
  }
  return FALLBACK;
}

const I18nContext = createContext(null);

/**
 * Module-level mirror of the active locale, kept in sync by I18nProvider,
 * so non-React modules (e.g. api.js prompt dialogs) can translate too.
 */
let activeLocale = detectLocale();

/** Translate outside of React components (uses the active locale). */
export function translate(key, vars) {
  const dict = MESSAGES[activeLocale] || MESSAGES[FALLBACK];
  let str = Object.prototype.hasOwnProperty.call(dict, key)
    ? dict[key]
    : (MESSAGES[FALLBACK][key] ?? key);
  if (vars) {
    for (const [name, value] of Object.entries(vars)) {
      str = str.split(`{${name}}`).join(String(value));
    }
  }
  return str;
}

export function I18nProvider({ children }) {
  const [locale, setLocaleState] = useState(detectLocale);

  const setLocale = useCallback((code) => {
    if (!MESSAGES[code]) return;
    setLocaleState(code);
    try { localStorage.setItem(LOCALE_KEY, code); } catch { /* storage unavailable */ }
  }, []);

  // Keep <html lang> and the module-level mirror in sync.
  useEffect(() => {
    activeLocale = locale;
    document.documentElement.lang = locale;
  }, [locale]);

  /** Translate a key with {var} interpolation; falls back to English, then the key. */
  const t = useCallback((key, vars) => {
    const dict = MESSAGES[locale] || MESSAGES[FALLBACK];
    let str = Object.prototype.hasOwnProperty.call(dict, key)
      ? dict[key]
      : (MESSAGES[FALLBACK][key] ?? key);
    if (vars) {
      for (const [name, value] of Object.entries(vars)) {
        str = str.split(`{${name}}`).join(String(value));
      }
    }
    return str;
  }, [locale]);

  /** Display label for a MODE_PRESETS key ("lineArt", "color", ...). */
  const presetLabel = useCallback((key) => t(`preset.${key}`), [t]);

  const value = useMemo(
    () => ({ locale, setLocale, t, presetLabel, supported: SUPPORTED_LOCALES }),
    [locale, setLocale, t, presetLabel]
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  const ctx = useContext(I18nContext);
  if (!ctx) throw new Error("useI18n must be used within <I18nProvider>");
  return ctx;
}

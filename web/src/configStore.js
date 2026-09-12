/**
 * Persistence for user-saved parameter configs (IndexedDB, with an
 * in-memory fallback when IndexedDB is unavailable).
 *
 * Saved configs are param presets shared across projects — they survive
 * app restarts. Only { id, label, params, savedAt } is stored; trace
 * results are NOT persisted (thumbnails are refilled from the trace
 * cache on load).
 */

const DB_NAME = "patternstudio-configs";
const DB_VERSION = 1;
const STORE = "configs";

let dbPromise = null;
let idbBroken = false;
const memFallback = new Map();

function idbSupported() {
  return typeof indexedDB !== "undefined" && !idbBroken;
}

function openDB() {
  if (!dbPromise) {
    dbPromise = new Promise((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, DB_VERSION);
      req.onupgradeneeded = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains(STORE)) {
          db.createObjectStore(STORE, { keyPath: "id" });
        }
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
      req.onblocked = () => reject(new Error("IndexedDB open blocked"));
    });
    // Allow retry after a failed open
    dbPromise.catch(() => { dbPromise = null; });
  }
  return dbPromise;
}

function reqP(req) {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function txDone(t) {
  return new Promise((resolve, reject) => {
    t.oncomplete = () => resolve();
    t.onerror = () => reject(t.error);
    t.onabort = () => reject(t.error);
  });
}

/** All saved configs, oldest first. Returns [] on any failure. */
export async function loadSavedConfigs() {
  if (!idbSupported()) return [...memFallback.values()];
  try {
    const db = await openDB();
    const t = db.transaction([STORE], "readonly");
    const rows = await reqP(t.objectStore(STORE).getAll());
    return rows.sort((a, b) => (a.savedAt || 0) - (b.savedAt || 0));
  } catch (e) {
    idbBroken = true;
    return [...memFallback.values()];
  }
}

/** Persist (or update) one saved config: { id, label, params, savedAt? }. */
export async function saveConfig(config) {
  const row = {
    id: config.id,
    label: config.label,
    params: config.params,
    savedAt: config.savedAt || Date.now(),
  };
  if (!idbSupported()) {
    memFallback.set(row.id, row);
    return;
  }
  try {
    const db = await openDB();
    const t = db.transaction([STORE], "readwrite");
    t.objectStore(STORE).put(row);
    await txDone(t);
  } catch (e) {
    idbBroken = true;
    memFallback.set(row.id, row);
  }
}

/** Remove a saved config by id. */
export async function deleteSavedConfig(id) {
  memFallback.delete(id);
  if (!idbSupported()) return;
  try {
    const db = await openDB();
    const t = db.transaction([STORE], "readwrite");
    t.objectStore(STORE).delete(id);
    await txDone(t);
  } catch (e) {
    idbBroken = true;
  }
}

/**
 * Trace result cache backed by IndexedDB (persists across app restarts),
 * with an in-memory fallback when IndexedDB is unavailable.
 *
 * Key = `${imageId}::${stableStringify(params)}`:
 * - same image + same params  -> hit, no recompute
 * - any param change          -> new key -> miss -> regenerate
 *   (old entries are kept, so switching params back hits again)
 */

const DB_NAME = "patternstudio-trace-cache";
const DB_VERSION = 1;
const RESULTS = "results";
const META = "meta";
const MAX_ENTRIES = 200;
const MAX_BYTES = 200 * 1024 * 1024; // ~200MB total budget

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
        if (!db.objectStoreNames.contains(RESULTS)) {
          db.createObjectStore(RESULTS, { keyPath: "key" });
        }
        if (!db.objectStoreNames.contains(META)) {
          db.createObjectStore(META, { keyPath: "key" });
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

/** Deterministic JSON serialization: object keys sorted recursively. */
export function stableStringify(value) {
  if (value === null || value === undefined) return "null";
  const t = typeof value;
  if (t === "string") return JSON.stringify(value);
  if (t === "number" || t === "boolean") return String(value);
  if (Array.isArray(value)) return "[" + value.map(stableStringify).join(",") + "]";
  if (t === "object") {
    const keys = Object.keys(value).sort();
    return "{" + keys.map((k) => JSON.stringify(k) + ":" + stableStringify(value[k])).join(",") + "}";
  }
  return "null";
}

export function makeKey(imageId, params) {
  return imageId + "::" + stableStringify(params || {});
}

function sizeOf(result) {
  return (result && result.svg ? result.svg : "").length;
}

/** Evict oldest entries until within MAX_ENTRIES / MAX_BYTES. */
async function evictIfNeeded() {
  const db = await openDB();
  const t = db.transaction([META, RESULTS], "readwrite");
  const metas = await reqP(t.objectStore(META).getAll());
  metas.sort((a, b) => (a.lastAccessed || 0) - (b.lastAccessed || 0));
  let count = metas.length;
  let bytes = metas.reduce((s, m) => s + (m.size || 0), 0);
  for (const m of metas) {
    if (count <= MAX_ENTRIES && bytes <= MAX_BYTES) break;
    t.objectStore(RESULTS).delete(m.key);
    t.objectStore(META).delete(m.key);
    count -= 1;
    bytes -= m.size || 0;
  }
  await txDone(t);
}

function memSet(key, result) {
  memFallback.set(key, { key, result, size: sizeOf(result), lastAccessed: Date.now() });
  if (memFallback.size > MAX_ENTRIES) {
    const oldest = memFallback.keys().next().value;
    memFallback.delete(oldest);
  }
}

/**
 * Get a cached trace result for (imageId, params), or null on miss.
 */
export async function getCachedTrace(imageId, params) {
  const key = makeKey(imageId, params);
  if (!idbSupported()) return (memFallback.get(key) || {}).result || null;
  try {
    const db = await openDB();
    const t = db.transaction([RESULTS, META], "readwrite");
    const rec = await reqP(t.objectStore(RESULTS).get(key));
    if (!rec) return null;
    // touch: refresh LRU position (best effort)
    t.objectStore(META).put({ key, size: rec.size || sizeOf(rec.result), lastAccessed: Date.now() });
    await txDone(t);
    return rec.result;
  } catch (e) {
    idbBroken = true;
    return (memFallback.get(key) || {}).result || null;
  }
}

/**
 * Store a trace result under (imageId, params).
 */
export async function setCachedTrace(imageId, params, result) {
  if (!result || !result.svg) return;
  const key = makeKey(imageId, params);
  const now = Date.now();
  const size = sizeOf(result);
  if (!idbSupported()) {
    memSet(key, result);
    return;
  }
  try {
    const db = await openDB();
    const t = db.transaction([RESULTS, META], "readwrite");
    t.objectStore(RESULTS).put({ key, imageId, result, size, lastAccessed: now });
    t.objectStore(META).put({ key, size, lastAccessed: now });
    await txDone(t);
    evictIfNeeded().catch(() => {});
  } catch (e) {
    idbBroken = true;
    memSet(key, result);
  }
}

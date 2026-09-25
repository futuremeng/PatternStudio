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

/**
 * Key for a vectorization-stage result. Depends only on the vectorize part
 * of the params (see VECTORIZE_PARAM_KEYS), so layering-only changes never
 * invalidate it. The `vec` tag keeps it distinct from processed-result keys.
 */
export function makeVectorizeKey(imageId, traceParams) {
  return imageId + "::vec::" + stableStringify(traceParams || {});
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

/** Get a cached result by key, or null on miss. */
async function getCachedByKey(key) {
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
 * Get a cached processed (layered) trace result for (imageId, fullParams),
 * or null on miss.
 */
export async function getCachedTrace(imageId, params) {
  return getCachedByKey(makeKey(imageId, params));
}

/**
 * Get a cached vectorization-stage result for (imageId, traceParams), or
 * null on miss.
 */
export async function getCachedVectorize(imageId, traceParams) {
  return getCachedByKey(makeVectorizeKey(imageId, traceParams));
}

/**
 * Delete every cached trace result that belongs to a project.
 *
 * Called when a project is removed. On desktop the project id is the folder
 * path, so re-opening the same folder (e.g. after replacing its images)
 * would otherwise hit stale cache entries and show old trace results.
 * Matches on the key prefix `${projectId}/` (key = imageId::params and
 * imageId = `${projectId}/${filename}`).
 */
export async function clearProjectCache(projectId) {
  if (!projectId) return;
  const prefix = projectId + "/";
  // In-memory fallback first (synchronous, always safe).
  for (const key of [...memFallback.keys()]) {
    if (key.startsWith(prefix)) memFallback.delete(key);
  }
  if (!idbSupported()) return;
  try {
    const db = await openDB();
    const t = db.transaction([META, RESULTS], "readwrite");
    // Scan the small meta store (not results, whose records hold the full
    // SVG strings) and delete matching keys from both stores.
    const metas = await reqP(t.objectStore(META).getAll());
    for (const m of metas) {
      if (typeof m.key === "string" && m.key.startsWith(prefix)) {
        t.objectStore(RESULTS).delete(m.key);
        t.objectStore(META).delete(m.key);
      }
    }
    await txDone(t);
  } catch (e) {
    idbBroken = true;
  }
}

/** Store a result under a cache key. */
async function setCachedByKey(key, imageId, result) {
  if (!result || !result.svg) return;
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

/**
 * Store a processed (layered) trace result under (imageId, fullParams).
 */
export async function setCachedTrace(imageId, params, result) {
  await setCachedByKey(makeKey(imageId, params), imageId, result);
}

/**
 * Store a vectorization-stage result under (imageId, traceParams).
 */
export async function setCachedVectorize(imageId, traceParams, result) {
  await setCachedByKey(makeVectorizeKey(imageId, traceParams), imageId, result);
}

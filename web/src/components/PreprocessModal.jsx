import React, { useEffect, useState, useCallback, useRef } from "react";
import { prepareSource, preprocessSource, saveEnhancedImage } from "../api.js";
import CompareViewer from "./CompareViewer.jsx";
import { IconMaximize, IconRestore } from "./icons.jsx";
import { useI18n } from "../i18n/index.jsx";

/**
 * Normalize an error thrown by the API layer into a display string. Tauri
 * command rejections are plain strings (not Error objects), so `e.message`
 * would be undefined there; the web path throws real Error objects.
 */
function errMsg(e) {
  return typeof e === "string" ? e : (e?.message || String(e));
}

/**
 * Full-screen-capable preprocessing modal: iterative digital-art enhancement
 * (Real-ESRGAN anime) with a thumbnail strip of every iteration. Each
 * "enhance" run takes the *currently selected* iteration as input (or the
 * original when the original thumbnail is selected), so the user can branch
 * from any earlier result. A rounds input (default 1) chains that many
 * enhancements per click; every round's result lands in the strip. Iteration
 * thumbnails carry a checkbox; "save as new image" writes every checked
 * iteration into the project folder (one file each) and the editor switches
 * to the last saved one.
 */
export default function PreprocessModal({ open, onClose, projectId, filename, imageUrl, onSaved }) {
  const { t } = useI18n();
  // Each entry: { id, source, previewUrl, width, height, elapsedMs, cached }
  const [iterations, setIterations] = useState([]);
  // Index into `iterations`, or -1 for the original image.
  const [selectedIdx, setSelectedIdx] = useState(-1);
  // Iteration indices checked for saving (independent of the compare
  // selection). Each checked iteration is saved as its own new image.
  const [checkedIdxs, setCheckedIdxs] = useState(() => new Set());
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState(null);
  const [compareMode, setCompareMode] = useState("side");
  // Number of enhancement rounds per "enhance" click (each round chains from
  // the previous one and lands in the thumbnail strip).
  const [steps, setSteps] = useState(1);
  // { current, total } while a run is in flight (drives the progress label).
  const [progress, setProgress] = useState(null);
  const [fullscreen, setFullscreen] = useState(false);
  // Natural size of the original (known once the left panel's img loads).
  const [origSize, setOrigSize] = useState({ w: 0, h: 0 });
  const runSeq = useRef(0);
  const iterationsRef = useRef([]);
  useEffect(() => { iterationsRef.current = iterations; }, [iterations]);

  // Reset (and revoke the old blob URLs) when the modal opens or the image
  // it is about changes.
  useEffect(() => {
    if (!open) return;
    setIterations(prev => {
      prev.forEach(it => { if (it.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(it.previewUrl); });
      return [];
    });
    setSelectedIdx(-1);
    setCheckedIdxs(new Set());
    setBusy(false);
    setSaving(false);
    setError(null);
    setCompareMode("side");
    setSteps(1);
    setProgress(null);
    setOrigSize({ w: 0, h: 0 });
    runSeq.current += 1; // invalidate any in-flight run
  }, [open, projectId, filename]);

  // Revoke the blob URLs of all iterations when the modal closes.
  useEffect(() => {
    if (!open) return;
    return () => {
      iterationsRef.current.forEach(it => {
        if (it.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(it.previewUrl);
      });
    };
  }, [open]);

  // Escape closes the modal (not while a run or save is in flight).
  useEffect(() => {
    if (!open) return;
    const onKey = (e) => { if (e.key === "Escape" && !busy && !saving) onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, saving, onClose]);

  /**
   * Run `steps` enhancement rounds: round 1 takes the currently selected
   * iteration (or the original), each following round chains from the
   * previous round's result. Every round lands in the thumbnail strip and
   * the latest one stays selected.
   */
  const runEnhance = useCallback(async () => {
    if (busy) return;
    const total = Math.max(1, Math.min(10, Math.floor(steps) || 1));
    const seq = ++runSeq.current;
    const startLen = iterations.length;
    setBusy(true);
    setProgress({ current: 0, total });
    setError(null);
    let input = selectedIdx >= 0
      ? iterations[selectedIdx].source
      : prepareSource(projectId, filename);
    try {
      for (let i = 0; i < total; i++) {
        setProgress({ current: i + 1, total });
        const out = await preprocessSource(input);
        if (seq !== runSeq.current) {
          // Superseded (modal closed / image switched): drop the blob.
          if (out.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(out.previewUrl);
          return;
        }
        setIterations(prev => [...prev, { ...out, id: `it-${seq}-${i}-${Date.now()}` }]);
        setSelectedIdx(startLen + i); // select the new latest iteration
        input = out.source; // next round chains from this result
      }
    } catch (e) {
      if (seq === runSeq.current) setError(t("preprocess.failed", { msg: errMsg(e) }));
    } finally {
      if (seq === runSeq.current) { setBusy(false); setProgress(null); }
    }
  }, [busy, steps, selectedIdx, iterations, projectId, filename, t]);

  // Stable base name derived from the original filename; each checked
  // iteration is saved as `{base}_enhanced_{n}.png` (n = its 1-based position
  // in the strip), so re-saving the same set replaces the same files.
  const baseName = filename.replace(/\.[^.]+$/, "");

  const toggleChecked = useCallback((i) => {
    setCheckedIdxs(prev => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  }, []);

  // Save every checked iteration as its own new image (sequentially, in strip
  // order), then switch the editor to the last saved one.
  const runSave = useCallback(async () => {
    if (checkedIdxs.size === 0 || saving) return;
    setSaving(true);
    setError(null);
    const idxs = [...checkedIdxs].sort((a, b) => a - b);
    try {
      let last = null;
      for (const i of idxs) {
        last = await saveEnhancedImage(projectId, iterations[i].source, `${baseName}_enhanced_${i + 1}.png`);
      }
      onSaved?.(last);
      onClose();
    } catch (e) {
      setError(t("preprocess.saveFailed", { msg: errMsg(e) }));
      setSaving(false);
    }
  }, [checkedIdxs, saving, iterations, projectId, baseName, onSaved, onClose, t]);

  if (!open) return null;

  const selected = selectedIdx >= 0 ? iterations[selectedIdx] : null;
  const canClose = !busy && !saving;
  // Busy label: plain "enhancing…" for a single round, "2/3…" for multi-round.
  const busyText = progress && progress.total > 1
    ? t("preprocess.enhancingRound", { current: progress.current, total: progress.total })
    : t("preprocess.enhancing");

  return (
    <div
      className="modal-overlay preprocess-modal-overlay"
      onClick={(e) => { if (e.target === e.currentTarget && canClose) onClose(); }}
    >
      <div className={`modal preprocess-modal${fullscreen ? " fullscreen" : ""}`}>
        <div className="modal-header">
          <h2>{t("preprocess.title")}</h2>
          <div className="preprocess-modal-actions">
            <button
              className="icon-btn"
              onClick={() => setFullscreen(f => !f)}
              title={fullscreen ? t("preprocess.exitFullscreen") : t("preprocess.fullscreen")}
              aria-label={fullscreen ? t("preprocess.exitFullscreen") : t("preprocess.fullscreen")}
            >
              {fullscreen ? <IconRestore /> : <IconMaximize />}
            </button>
            <button className="icon-btn" onClick={onClose} disabled={!canClose} aria-label={t("close")}>×</button>
          </div>
        </div>

        <div className="preprocess-modal-body">
          <CompareViewer
            leftUrl={imageUrl}
            right={selected ? { imageUrl: selected.previewUrl } : null}
            contentSize={selected ? { w: selected.width, h: selected.height } : origSize}
            onImageLoad={(w, h) => setOrigSize({ w, h })}
            compareMode={compareMode}
            onCompareModeChange={setCompareMode}
            busy={busy}
            leftLabel={t("editor.original")}
            rightLabel={t("preprocess.enhanced")}
            busyLabel={busyText}
            placeholder={t("preprocess.startHint")}
          />
          <div className="preprocess-thumbs">
            <button
              className={`preprocess-thumb${selectedIdx === -1 ? " active" : ""}`}
              onClick={() => setSelectedIdx(-1)}
              title={t("preprocess.original")}
            >
              <img src={imageUrl} alt="" />
              <span className="preprocess-thumb-label">{t("preprocess.original")}</span>
            </button>
            {iterations.map((it, i) => {
              const checked = checkedIdxs.has(i);
              return (
                <button
                  key={it.id}
                  className={`preprocess-thumb${selectedIdx === i ? " active" : ""}`}
                  onClick={() => setSelectedIdx(i)}
                  title={t("preprocess.iteration", { n: i + 1 }) + (it.cached ? ` · ${t("preprocess.cached")}` : "")}
                >
                  <img src={it.previewUrl} alt="" />
                  <span
                    className={`preprocess-thumb-check${checked ? " checked" : ""}`}
                    onClick={(e) => { e.stopPropagation(); toggleChecked(i); }}
                    title={t("preprocess.checkSave")}
                    aria-label={t("preprocess.checkSave")}
                  >
                    {checked && "✓"}
                  </span>
                  {it.cached && <span className="preprocess-thumb-cached">⚡</span>}
                  <span className="preprocess-thumb-label">{i + 1}</span>
                </button>
              );
            })}
          </div>
        </div>

        {error && <div className="error preprocess-modal-error">{error}</div>}

        <div className="modal-footer preprocess-modal-footer">
          <input
            className="preprocess-steps"
            type="number"
            min={1}
            max={10}
            value={steps}
            onChange={(e) => {
              const v = parseInt(e.target.value, 10);
              setSteps(Number.isFinite(v) ? Math.max(1, Math.min(10, v)) : 1);
            }}
            disabled={busy || saving}
            title={t("preprocess.steps")}
            aria-label={t("preprocess.steps")}
          />
          <button onClick={runEnhance} disabled={busy || saving}>
            {busy ? busyText : t("preprocess.enhance")}
          </button>
          <button className="primary" onClick={runSave} disabled={checkedIdxs.size === 0 || busy || saving}>
            {saving
              ? t("preprocess.saving")
              : checkedIdxs.size > 1
                ? t("preprocess.saveAsNewMany", { n: checkedIdxs.size })
                : t("preprocess.saveAsNew")}
          </button>
        </div>
      </div>
    </div>
  );
}

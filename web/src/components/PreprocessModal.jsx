import React, { useEffect, useState, useCallback, useRef } from "react";
import { prepareSource, preprocessSource, redrawSource, saveProjectImage } from "../api.js";
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
 * Full-screen-capable image-optimization modal with two stages:
 *
 * - "enhance" — iterative digital-art enhancement (Real-ESRGAN anime). Each
 *   run takes the *currently selected* iteration as input (or the original
 *   when the original thumbnail is selected), so the user can branch from any
 *   earlier result. A rounds input (default 1) chains that many enhancements
 *   per click.
 * - "redraw" — Qwen-Image prompt-driven restyle. A preset fills an editable
 *   prompt; each click restyles the *original* image (never an iteration)
 *   and appends the result to the strip.
 *
 * Both stages share the thumbnail strip of every iteration. Every iteration
 * is auto-persisted into the project (`.patternstudio/generated/`) with a
 * lineage record, so the project's list view can show the full derived tree
 * (and trace each image back to its original). Iteration thumbnails carry a
 * checkbox; "save as new image" confirms every checked iteration into the
 * project folder (one file each, `_enhanced_n` / `_redrawn_n`, marked
 * `confirmed` in the lineage) and the editor switches to the last saved one.
 */
export default function PreprocessModal({ open, onClose, projectId, filename, imageUrl, currentLineage, isGenerated, onSaved }) {
  const { t } = useI18n();
  // Root original of this image: when the modal is about a derived image
  // (itself confirmed from an earlier original), trace back to its root so
  // new iterations hang off the same tree.
  const rootName = currentLineage?.root || filename;
  // Stable base name derived from the current filename; confirmed iterations
  // are saved as `{base}_{enhanced|redrawn}_{n}.png` (n = 1-based strip
  // position), so re-saving the same set replaces the same files.
  const baseName = filename.replace(/\.[^.]+$/, "");
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
  // Stage: "enhance" (Real-ESRGAN) or "redraw" (Qwen-Image). Both share the
  // iteration strip, but a redraw always starts from the original image.
  const [mode, setMode] = useState("enhance");
  // Redraw preset key (fills the prompt textarea; "custom" keeps it as-is).
  const [preset, setPreset] = useState("lineart");
  // Editable redraw prompt (pre-filled from the selected preset).
  const [prompt, setPrompt] = useState("");
  // Optional seed for reproducible redraws ("" = the model chooses).
  const [seedText, setSeedText] = useState("");
  // { current, total } while a run is in flight (drives the progress label).
  const [progress, setProgress] = useState(null);
  const [fullscreen, setFullscreen] = useState(false);
  // Natural size of the original (known once the left panel's img loads).
  const [origSize, setOrigSize] = useState({ w: 0, h: 0 });
  const runSeq = useRef(0);
  const iterationsRef = useRef([]);
  useEffect(() => { iterationsRef.current = iterations; }, [iterations]);
  // Ref to the currently selected thumbnail so the slider can keep it in
  // view while the strip scrolls horizontally.
  const activeThumbRef = useRef(null);
  useEffect(() => {
    activeThumbRef.current?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [selectedIdx]);

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
    setMode("enhance");
    setPreset("lineart");
    setPrompt(t("redraw.preset.lineart"));
    setSeedText("");
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
   * Auto-persist a generated iteration into the project (`.patternstudio/
   * generated/`) with its lineage record, so the project's list view can show
   * the full derived tree. Returns { genName, entry } on success, or null
   * when persistence failed — the iteration stays usable in the strip, just
   * untracked in the lineage tree.
   */
  const persistIteration = useCallback(async (source, kind, parent, meta, pos) => {
    const suffix = kind === "redraw" ? "redrawn" : "enhanced";
    const genName = `${baseName}_${suffix}_${pos}_${Date.now()}.png`;
    const entry = {
      root: rootName,
      parent,
      kind,
      confirmed: false,
      confirmedFile: null,
      createdAt: new Date().toISOString(),
      meta,
    };
    try {
      await saveProjectImage(projectId, source, genName, { generated: true, lineage: entry });
      return { genName, entry };
    } catch (e) {
      console.warn("Failed to auto-save iteration:", e);
      return null;
    }
  }, [baseName, rootName, projectId]);

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
      : prepareSource(projectId, filename, { generated: isGenerated });
    // Lineage parent of round 1: the selected iteration's persisted file
    // (when auto-saved), else the image this modal is about.
    let parent = selectedIdx >= 0
      ? (iterations[selectedIdx].genName || filename)
      : filename;
    try {
      for (let i = 0; i < total; i++) {
        setProgress({ current: i + 1, total });
        const out = await preprocessSource(input);
        if (seq !== runSeq.current) {
          // Superseded (modal closed / image switched): drop the blob.
          if (out.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(out.previewUrl);
          return;
        }
        const persisted = await persistIteration(out.source, "enhance", parent, {}, startLen + i + 1);
        setIterations(prev => [...prev, {
          ...out, kind: "enhance", id: `it-${seq}-${i}-${Date.now()}`,
          genName: persisted?.genName || null, entry: persisted?.entry || null,
        }]);
        setSelectedIdx(startLen + i); // select the new latest iteration
        input = out.source; // next round chains from this result
        if (persisted) parent = persisted.genName; // chain lineage from the persisted file
      }
    } catch (e) {
      if (seq === runSeq.current) setError(t("preprocess.failed", { msg: errMsg(e) }));
    } finally {
      if (seq === runSeq.current) { setBusy(false); setProgress(null); }
    }
  }, [busy, steps, selectedIdx, iterations, projectId, filename, isGenerated, t, persistIteration]);

  // Selecting a preset fills the prompt textarea ("custom" keeps the text).
  const applyPreset = useCallback((key) => {
    setPreset(key);
    if (key !== "custom") setPrompt(t(`redraw.preset.${key}`));
  }, [t]);

  /**
   * Run one Qwen-Image redraw: always takes the *original* image as input
   * (unlike enhance, which chains from the selected iteration), restyles it
   * with the prompt, and appends the result to the strip. Each click is a
   * fresh generation.
   */
  const runRedraw = useCallback(async () => {
    if (busy) return;
    const p = prompt.trim();
    if (!p) {
      setError(t("redraw.emptyPrompt"));
      return;
    }
    const seq = ++runSeq.current;
    const startLen = iterations.length;
    setBusy(true);
    setProgress({ current: 1, total: 1 });
    setError(null);
    // Redraw always starts from the original image, never from an iteration.
    const input = prepareSource(projectId, filename, { generated: isGenerated });
    const seed = seedText.trim() === "" ? null : Number(seedText);
    try {
      const out = await redrawSource(input, p, Number.isFinite(seed) ? seed : null);
      if (seq !== runSeq.current) {
        // Superseded (modal closed / image switched): drop the blob.
        if (out.previewUrl?.startsWith("blob:")) URL.revokeObjectURL(out.previewUrl);
        return;
      }
      const persisted = await persistIteration(
        out.source, "redraw", filename,
        { prompt: p, seed: Number.isFinite(seed) ? seed : null },
        startLen + 1,
      );
      setIterations(prev => [...prev, {
        ...out, kind: "redraw", id: `it-${seq}-${startLen}-${Date.now()}`,
        genName: persisted?.genName || null, entry: persisted?.entry || null,
      }]);
      setSelectedIdx(startLen); // select the new latest iteration
    } catch (e) {
      if (seq === runSeq.current) setError(t("redraw.failed", { msg: errMsg(e) }));
    } finally {
      if (seq === runSeq.current) { setBusy(false); setProgress(null); }
    }
  }, [busy, prompt, seedText, iterations, projectId, filename, isGenerated, t, persistIteration]);

  // Stable base name derived from the original filename; each checked
  // iteration is saved as `{base}_enhanced_{n}.png` (n = its 1-based position
  // in the strip), so re-saving the same set replaces the same files.

  const toggleChecked = useCallback((i) => {
    setCheckedIdxs(prev => {
      const next = new Set(prev);
      if (next.has(i)) next.delete(i);
      else next.add(i);
      return next;
    });
  }, []);

  // Confirm every checked iteration into the project folder (sequentially, in
  // strip order), marking its lineage entry `confirmed`, then switch the
  // editor to the last saved one.
  const runSave = useCallback(async () => {
    if (checkedIdxs.size === 0 || saving) return;
    setSaving(true);
    setError(null);
    const idxs = [...checkedIdxs].sort((a, b) => a - b);
    try {
      let last = null;
      for (const i of idxs) {
        const suffix = iterations[i].kind === "redraw" ? "redrawn" : "enhanced";
        const confirmedName = `${baseName}_${suffix}_${i + 1}.png`;
        // Reuse the entry recorded at auto-persist time (root/parent/kind/
        // meta), now marked confirmed; fall back to no lineage when the
        // iteration was never persisted.
        const entry = iterations[i].entry
          ? { ...iterations[i].entry, confirmed: true, confirmedFile: confirmedName }
          : null;
        last = await saveProjectImage(projectId, iterations[i].source, confirmedName, {
          generated: false,
          lineage: entry,
          lineageKey: iterations[i].genName || null,
        });
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
  // Busy label: plain "enhancing…" for a single round, "2/3…" for multi-round;
  // redraw runs show the redraw label.
  const busyText = progress && progress.total > 1
    ? t("preprocess.enhancingRound", { current: progress.current, total: progress.total })
    : mode === "redraw"
      ? t("redraw.running")
      : t("preprocess.enhancing");

  // Note: clicking the dark overlay area deliberately does NOT close this
  // modal — a stray click would discard the in-progress enhancement state.
  // Close via the × button or the Escape key instead.
  return (
    <div className="modal-overlay preprocess-modal-overlay">
      <div className={`modal preprocess-modal${fullscreen ? " fullscreen" : ""}`}>
        <div className="modal-header">
          <h2>{t("preprocess.title")}</h2>
          <div className="preprocess-tabs" role="tablist">
            <button
              role="tab"
              aria-selected={mode === "enhance"}
              className={mode === "enhance" ? "active" : ""}
              onClick={() => setMode("enhance")}
            >
              {t("preprocess.tabEnhance")}
            </button>
            <button
              role="tab"
              aria-selected={mode === "redraw"}
              className={mode === "redraw" ? "active" : ""}
              onClick={() => setMode("redraw")}
            >
              {t("preprocess.tabRedraw")}
            </button>
          </div>
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
            rightLabel={selected?.kind === "redraw" ? t("redraw.result") : t("preprocess.enhanced")}
            busyLabel={busyText}
            placeholder={mode === "redraw" ? t("redraw.startHint") : t("preprocess.startHint")}
          />
          <div className="preprocess-thumbs">
            <button
              ref={selectedIdx === -1 ? activeThumbRef : null}
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
                  ref={selectedIdx === i ? activeThumbRef : null}
                  className={`preprocess-thumb${selectedIdx === i ? " active" : ""}`}
                  onClick={() => setSelectedIdx(i)}
                  title={
                    (it.kind === "redraw"
                      ? t("redraw.iteration", { n: i + 1 })
                      : t("preprocess.iteration", { n: i + 1 }))
                    + (it.cached ? ` · ${t("preprocess.cached")}` : "")
                    + (it.prompt ? `\n${it.prompt}` : "")
                  }
                >
                  <img src={it.previewUrl} alt="" />
                  <span className={`preprocess-thumb-kind ${it.kind === "redraw" ? "redraw" : "enhance"}`}>
                    {it.kind === "redraw" ? t("redraw.badge") : t("preprocess.badge")}
                  </span>
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
          {/* Iteration scrubber: 1 = first enhancement, N = last. The
              original is only reachable via its thumbnail (selecting it
              leaves the right compare panel empty), so the slider starts
              at the first enhancement. Dragging it swaps the compare
              preview instantly, so fast scrubbing lets the user flick
              through results to pick the best one. */}
          <div className="preprocess-slider-row">
            <input
              className="preprocess-slider"
              type="range"
              min={1}
              max={Math.max(1, iterations.length)}
              step={1}
              value={selectedIdx >= 0 ? selectedIdx + 1 : 1}
              onChange={(e) => setSelectedIdx(parseInt(e.target.value, 10) - 1)}
              disabled={iterations.length === 0}
              title={t("preprocess.slider")}
              aria-label={t("preprocess.slider")}
            />
            <span className="preprocess-slider-val">
              {t("preprocess.iteration", { n: selectedIdx >= 0 ? selectedIdx + 1 : 1 })}
            </span>
          </div>
        </div>

        {error && <div className="error preprocess-modal-error">{error}</div>}

        {mode === "redraw" && (
          <div className="redraw-controls">
            <div className="redraw-controls-row">
              <select
                className="redraw-preset"
                value={preset}
                onChange={(e) => applyPreset(e.target.value)}
                disabled={busy || saving}
                title={t("redraw.presetLabel")}
                aria-label={t("redraw.presetLabel")}
              >
                {/* 选项只显示短标签;完整提示词由 applyPreset 填入下方文本框 */}
                {["lineart", "gongbi", "ink", "flat", "decorative", "custom"].map((key) => (
                  <option key={key} value={key}>
                    {t(`redraw.presetName.${key}`)}
                  </option>
                ))}
              </select>
              <input
                className="redraw-seed"
                type="number"
                value={seedText}
                onChange={(e) => setSeedText(e.target.value)}
                disabled={busy || saving}
                placeholder={t("redraw.seedPlaceholder")}
                title={t("redraw.seedPlaceholder")}
                aria-label={t("redraw.seedPlaceholder")}
              />
            </div>
            <textarea
              className="redraw-prompt"
              rows={3}
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              disabled={busy || saving}
              placeholder={t("redraw.promptPlaceholder")}
              title={t("redraw.promptLabel")}
              aria-label={t("redraw.promptLabel")}
            />
          </div>
        )}
        <div className="modal-footer preprocess-modal-footer">
          {mode === "enhance" && (
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
          )}
          {mode === "enhance" ? (
            <button onClick={runEnhance} disabled={busy || saving}>
              {busy ? busyText : t("preprocess.enhance")}
            </button>
          ) : (
            <button onClick={runRedraw} disabled={busy || saving}>
              {busy ? t("redraw.running") : t("redraw.run")}
            </button>
          )}
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

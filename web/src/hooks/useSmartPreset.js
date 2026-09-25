import { useEffect, useState, useCallback } from "react";
import { prepareSource, smartSource, familyFromParams } from "../api.js";
import { errMsg } from "../editorUtils.js";

/**
 * Smart Preset tuning: search for the best parameters *within the preset
 * family the user is currently working in*. The winning params are pushed
 * through `applyParams`, which triggers the normal auto-trace, so the
 * preview updates for free.
 */
export function useSmartPreset({
  imageUrl, params, projectId, filename, imageId, isGenerated,
  t, applyParams, setError, setErrorKind,
}) {
  const [smartInfo, setSmartInfo] = useState(null); // last Smart Preset decision
  const [smartBusy, setSmartBusy] = useState(false);

  const runSmart = useCallback(async () => {
    if (!imageUrl || smartBusy) return;
    setSmartBusy(true);
    setSmartInfo(null);
    setError(null);
    setErrorKind(null);
    try {
      const source = prepareSource(projectId, filename, { generated: isGenerated });
      const family = familyFromParams(params);
      const decision = await smartSource(source, family);
      setSmartInfo(decision);
      if (decision?.params) applyParams(decision.params);
    } catch (e) {
      setError(t("editor.smartFailed", { msg: errMsg(e) }));
      setErrorKind("smart");
    } finally {
      setSmartBusy(false);
    }
  }, [imageUrl, smartBusy, projectId, filename, isGenerated, params, t, applyParams, setError, setErrorKind]);

  /**
   * Apply the preset-default result (from the smart comparison) instead of
   * the smart winner, so the user can fall back to the known-good preset if
   * the tuned result doesn't look right.
   */
  const applyPresetResult = useCallback((presetParams) => {
    if (presetParams) applyParams(presetParams);
  }, [applyParams]);

  // Clear the smart banner when switching to a different image.
  useEffect(() => { setSmartInfo(null); }, [imageId]);

  return { smartInfo, setSmartInfo, smartBusy, runSmart, applyPresetResult };
}

import React, { useEffect, useState, useCallback, useRef } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { getImageUrl, traceImage, downloadSvg, MODE_PRESETS } from "../api.js";
import ParameterPanel from "../components/ParameterPanel.jsx";
import LayerPanel from "../components/LayerPanel.jsx";
import ModeStrip from "../components/ModeStrip.jsx";

const DEFAULT_PARAMS = {
  preset: "poster",
  clustering: "color",
  hierarchical: "stacked",
  colorPrecision: 6,
  layerDifference: 16,
  filterSpeckle: 4,
  mode: "spline",
  simplify: 0.5,
  pathPrecision: 2,
  maxColors: null,
  lineArt: false,
  layers: 0,
};

export default function ImageEditorPage() {
  const { projectId, filename } = useParams();
  const navigate = useNavigate();
  const [imageUrl, setImageUrl] = useState(null);
  const [params, setParams] = useState(DEFAULT_PARAMS);
  const [result, setResult] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [tab, setTab] = useState("params");
  const [activeMode, setActiveMode] = useState("color");
  const [modeResults, setModeResults] = useState({});
  const fileRef = useRef(null);

  useEffect(() => {
    (async () => {
      try {
        const url = await getImageUrl(projectId, filename);
        setImageUrl(url);
        // Auto-trace with default params
        const resp = await fetch(url);
        const blob = await resp.blob();
        const file = new File([blob], filename, { type: blob.type });
        await doTrace(file, DEFAULT_PARAMS);
      } catch (e) {
        setError(e.message);
      }
    })();
  }, [projectId, filename]);

  const doTrace = useCallback(async (file, traceParams) => {
    setBusy(true);
    setError(null);
    try {
      const res = await traceImage(file, traceParams);
      setResult(res);
    } catch (e) {
      setError(`Trace 失败: ${e.message}`);
    } finally {
      setBusy(false);
    }
  }, []);

  const onTrace = useCallback(async () => {
    if (!imageUrl) return;
    try {
      const resp = await fetch(imageUrl);
      const blob = await resp.blob();
      const file = new File([blob], filename, { type: blob.type });
      await doTrace(file, params);
    } catch (e) {
      setError(e.message);
    }
  }, [imageUrl, filename, params, doTrace]);

  const onModeSelect = useCallback(async (modeKey) => {
    setActiveMode(modeKey);
    if (modeResults[modeKey]) return;
    if (!imageUrl) return;
    try {
      const preset = MODE_PRESETS[modeKey];
      const merged = { ...params, ...preset.params };
      const resp = await fetch(imageUrl);
      const blob = await resp.blob();
      const file = new File([blob], filename, { type: blob.type });
      const res = await traceImage(file, merged);
      setModeResults(prev => ({ ...prev, [modeKey]: res }));
      setResult(res);
    } catch (e) {
      setError(`模式 "${preset.label}" 处理失败: ${e.message}`);
    }
  }, [imageUrl, filename, params, modeResults]);

  const onDownload = useCallback(() => {
    if (result?.svg) {
      downloadSvg(result.svg, filename.replace(/\.\w+$/, "") + ".svg");
    }
  }, [result, filename]);

  return (
    <div className="editor-page">
      <div className="detail-header">
        <button className="back-btn" onClick={() => navigate(`/project/${encodeURIComponent(projectId)}`)}>← 返回</button>
        <h2>{filename}</h2>
        <div className="tabs">
          <button className={tab === "params" ? "active" : ""} onClick={() => setTab("params")}>参数</button>
          <button className={tab === "layers" ? "active" : ""} onClick={() => setTab("layers")}>图层</button>
        </div>
        <button className="primary" onClick={onTrace} disabled={busy}>
          {busy ? "处理中..." : "重新 Trace"}
        </button>
        <button onClick={onDownload} disabled={!result?.svg}>下载 SVG</button>
      </div>

      <div className="editor-main">
        <aside className="editor-sidebar">
          {tab === "params" ? (
            <ParameterPanel params={params} onChange={setParams} />
          ) : (
            <LayerPanel layers={result?.layers || []} />
          )}
        </aside>

        <div className="editor-content">
          <div className="before-after">
            <div className="ba-panel">
              <span className="label">原图</span>
              {imageUrl ? <img src={imageUrl} alt="original" /> : <div className="placeholder">加载中</div>}
            </div>
            <div className="ba-panel">
              <span className="label">SVG 结果</span>
              {result?.svg ? (
                <div className="svg-frame" dangerouslySetInnerHTML={{ __html: result.svg }} />
              ) : (
                <div className="placeholder">{busy ? "处理中..." : "点击 Trace 开始"}</div>
              )}
            </div>
          </div>
          {error && <div className="error" style={{padding: "0 1rem"}}>{error}</div>}
        </div>
      </div>

      <ModeStrip
        activeMode={activeMode}
        onModeSelect={onModeSelect}
        modeResults={modeResults}
        imageUrl={imageUrl}
      />
    </div>
  );
}
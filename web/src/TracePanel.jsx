import React, { useRef, useState } from "react";
import { traceImage } from "./api.js";
import LayerEditor from "./LayerEditor.jsx";

const PRESETS = ["poster", "bw", "photo"];
const RESOLUTIONS = ["coarse", "medium", "fine"];
const SIMPLIFIES = ["minimal", "low", "medium", "high", "maximum"];

export default function TracePanel() {
  const [file, setFile] = useState(null);
  const [dragOver, setDragOver] = useState(false);
  const [params, setParams] = useState({
    preset: "poster",
    resolution: "medium",
    simplify: "low",
    layers: 0,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [result, setResult] = useState(null); // {svg, width, height, shape_count, layer_count}
  const inputRef = useRef(null);

  const set = (key) => (e) => setParams((p) => ({ ...p, [key]: e.target.value }));

  function onFile(f) {
    if (!f) return;
    setError(null);
    setFile(f);
    setResult(null);
  }

  async function onTrace() {
    if (!file) return;
    setBusy(true);
    setError(null);
    try {
      const res = await traceImage(file, { ...params, layers: Number(params.layers) });
      setResult(res);
    } catch (e) {
      setError(`Trace 失败: ${e.message}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="split">
      <aside className="sidebar">
        <div
          className={`dropzone ${dragOver ? "over" : ""}`}
          onDragOver={(e) => {
            e.preventDefault();
            setDragOver(true);
          }}
          onDragLeave={() => setDragOver(false)}
          onDrop={(e) => {
            e.preventDefault();
            setDragOver(false);
            onFile(e.dataTransfer.files[0]);
          }}
          onClick={() => inputRef.current?.click()}
        >
          <input
            ref={inputRef}
            type="file"
            accept="image/*"
            hidden
            onChange={(e) => onFile(e.target.files[0])}
          />
          {file ? (
            <>
              <img
                className="thumb"
                src={URL.createObjectURL(file)}
                alt={file.name}
              />
              <div className="fname" title={file.name}>
                {file.name}
              </div>
            </>
          ) : (
            <div className="dz-hint">拖拽图片到这里，或点击选择</div>
          )}
        </div>

        <div className="params">
          <label>
            Preset
            <select value={params.preset} onChange={set("preset")}>
              {PRESETS.map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label>
            Resolution
            <select value={params.resolution} onChange={set("resolution")}>
              {RESOLUTIONS.map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label>
            Simplify
            <select value={params.simplify} onChange={set("simplify")}>
              {SIMPLIFIES.map((p) => (
                <option key={p}>{p}</option>
              ))}
            </select>
          </label>
          <label>
            自动分层 (0 = 单层)
            <input
              type="number"
              min="0"
              max="16"
              value={params.layers}
              onChange={set("layers")}
            />
          </label>
          <button className="primary" disabled={!file || busy} onClick={onTrace}>
            {busy ? "处理中..." : "Trace"}
          </button>
          {error && <div className="error">{error}</div>}
        </div>
      </aside>

      <section className="content">
        {result ? (
          <LayerEditor
            key={result.svg.length + result.layer_count}
            svg={result.svg}
            width={result.width}
            height={result.height}
            stats={{ shapes: result.shape_count, layers: result.layer_count }}
          />
        ) : (
          <div className="placeholder">
            {busy ? "正在处理..." : "上传图片并点击 Trace，结果会显示在这里"}
          </div>
        )}
      </section>
    </div>
  );
}

import React, { useEffect, useState } from "react";
import { health } from "./api.js";
import TracePanel from "./TracePanel.jsx";
import PatternPanel from "./PatternPanel.jsx";

export default function App() {
  const [tab, setTab] = useState("trace");
  const [server, setServer] = useState(null);

  useEffect(() => {
    health()
      .then(setServer)
      .catch(() => setServer({ status: "offline" }));
  }, []);

  return (
    <div className="app">
      <header className="header">
        <h1>PatternStudio</h1>
        <span className={`badge ${server?.status === "ok" ? "ok" : "off"}`}>
          {server?.status === "ok" ? `API v${server.version}` : "API offline"}
        </span>
        <nav className="tabs">
          <button className={tab === "trace" ? "active" : ""} onClick={() => setTab("trace")}>
            图片转 SVG
          </button>
          <button className={tab === "pattern" ? "active" : ""} onClick={() => setTab("pattern")}>
            纹样生成
          </button>
        </nav>
      </header>
      <main className="main">
        {tab === "trace" ? <TracePanel /> : <PatternPanel />}
      </main>
    </div>
  );
}

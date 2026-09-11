import React, { useEffect, useState } from "react";
import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import { health } from "./api.js";
import ProjectListPage from "./pages/ProjectListPage.jsx";
import ProjectDetailPage from "./pages/ProjectDetailPage.jsx";
import ImageEditorPage from "./pages/ImageEditorPage.jsx";

export default function App() {
  const [server, setServer] = useState(null);

  useEffect(() => {
    health().then(setServer).catch(() => setServer({ status: "offline" }));
  }, []);

  return (
    <div className="app">
      <header className="header">
        <h1>PatternStudio</h1>
        <span className={`badge ${server?.status === "ok" ? "ok" : "off"}`}>
          {server?.status === "ok" ? `API v${server.version}` : "API offline"}
        </span>
      </header>
      <main className="main">
        <HashRouter>
          <Routes>
            <Route path="/" element={<ProjectListPage />} />
            <Route path="/project/:projectId" element={<ProjectDetailPage />} />
            <Route path="/project/:projectId/image/:filename" element={<ImageEditorPage />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </HashRouter>
      </main>
    </div>
  );
}

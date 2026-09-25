import React from "react";
import { HashRouter, Routes, Route, Navigate } from "react-router-dom";
import ProjectListPage from "./pages/ProjectListPage.jsx";
import ProjectDetailPage from "./pages/ProjectDetailPage.jsx";
import LineageTreePage from "./pages/LineageTreePage.jsx";
import ImageEditorPage from "./pages/ImageEditorPage.jsx";

export default function App() {
  return (
    <div className="app">
      <main className="main">
        <HashRouter>
          <Routes>
            <Route path="/" element={<ProjectListPage />} />
            <Route path="/project/:projectId" element={<ProjectDetailPage />} />
            <Route path="/project/:projectId/tree/:filename" element={<LineageTreePage />} />
            <Route path="/project/:projectId/image/:filename" element={<ImageEditorPage />} />
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </HashRouter>
      </main>
    </div>
  );
}

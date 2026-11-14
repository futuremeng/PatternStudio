import React, { useCallback, useEffect, useSyncExternalStore } from "react";
import { HashRouter, Routes, Route, Navigate, useLocation, useNavigate } from "react-router-dom";
import { isTauri, openProject } from "./api.js";
import { subscribeSettings, getSettingsOpen, openSettings, closeSettings } from "./settingsStore.js";
import { toggleTheme } from "./theme.js";
import SettingsModal from "./components/SettingsModal.jsx";
import ProjectListPage from "./pages/ProjectListPage.jsx";
import ProjectDetailPage from "./pages/ProjectDetailPage.jsx";
import LineageTreePage from "./pages/LineageTreePage.jsx";
import ImageEditorPage from "./pages/ImageEditorPage.jsx";

/**
 * Broadcast an app-level command (from the native menu bar or a shortcut) to
 * whoever can act on it (e.g. the SVG editor listens for undo/redo).
 */
export function dispatchAppCommand(cmd) {
  window.dispatchEvent(new CustomEvent("ps:command", { detail: { cmd } }));
}

/** Everything that needs router context lives here, inside the HashRouter. */
function Shell() {
  const navigate = useNavigate();
  const location = useLocation();
  const settingsOpen = useSyncExternalStore(subscribeSettings, getSettingsOpen, getSettingsOpen);

  const openProjectFlow = useCallback(async () => {
    try {
      const project = await openProject();
      navigate(`/project/${encodeURIComponent(project.id)}`);
    } catch (e) {
      if (e?.message !== "cancelled") console.error(e);
    }
  }, [navigate]);

  // Global shortcuts (⌘, settings, ⌘O open) + native menu-bar commands.
  useEffect(() => {
    const run = async (cmd) => {
      if (cmd === "settings") openSettings();
      else if (cmd === "toggle-appearance") toggleTheme();
      else if (cmd === "open-folder") await openProjectFlow();
      else dispatchAppCommand(cmd); // undo / redo → whoever owns the editor
    };
    const onKey = (e) => {
      const mod = e.metaKey || e.ctrlKey;
      if (!mod) return;
      if (e.key === ",") {
        e.preventDefault();
        openSettings();
      } else if ((e.key === "o" || e.key === "O") && !e.shiftKey) {
        e.preventDefault();
        openProjectFlow();
      }
    };
    window.addEventListener("keydown", onKey);
    let unlisten = null;
    let disposed = false;
    if (isTauri()) {
      import("@tauri-apps/api/event").then(({ listen }) =>
        listen("menu-command", (ev) => run(ev.payload?.cmd))
      ).then((u) => {
        if (disposed) u();
        else unlisten = u;
      });
    }
    return () => {
      disposed = true;
      window.removeEventListener("keydown", onKey);
      if (unlisten) unlisten();
    };
  }, [openProjectFlow]);

  // Finder → window drag & drop: image files land in the current project.

  // Mac convention: the window title names the current document/project.
  useEffect(() => {
    if (!isTauri()) return;
    const m = location.pathname.match(/^\/project\/([^/]+)/);
    const title = m
      ? `${decodeURIComponent(m[1]).split("/").pop()} — PatternStudio`
      : "PatternStudio";
    import("@tauri-apps/api/window").then(({ getCurrentWindow }) =>
      getCurrentWindow().setTitle(title)
    ).catch(() => { /* title is cosmetic; ignore */ });
  }, [location.pathname]);

  return (
    <>
      <Routes>
        <Route path="/" element={<ProjectListPage />} />
        <Route path="/project/:projectId" element={<ProjectDetailPage />} />
        <Route path="/project/:projectId/tree/:filename" element={<LineageTreePage />} />
        <Route path="/project/:projectId/image/:filename" element={<ImageEditorPage />} />
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
      <SettingsModal open={settingsOpen} onClose={closeSettings} />
    </>
  );
}

export default function App() {
  return (
    <div className="app">
      <main className="main">
        <HashRouter>
          <Shell />
        </HashRouter>
      </main>
    </div>
  );
}

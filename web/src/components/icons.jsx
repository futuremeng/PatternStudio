import React from "react";

// --- Shared UI icons (inline SVG, 16px) ---
// Extracted from ImageEditorPage so CompareViewer / PreprocessModal can reuse
// the same glyphs as the editor's preview control column.

export const IconMaximize = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M8 3H5a2 2 0 0 0-2 2v3M16 3h3a2 2 0 0 1 2 2v3M8 21H5a2 2 0 0 1-2-2v-3M16 21h3a2 2 0 0 0 2-2v-3" />
  </svg>
);

export const IconRestore = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M8 3v3a2 2 0 0 1-2 2H3M21 8h-3a2 2 0 0 1-2-2V3M3 16h3a2 2 0 0 1 2 2v3M16 21v-3a2 2 0 0 1 2-2h3" />
  </svg>
);

export const IconSelect = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
    <path d="M4 4l0 14 4.5-4.5 3 6.5 2.5-1.2-3-6.3 6 0z" />
  </svg>
);

export const IconCompare = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor">
    <rect x="11" y="4" width="2" height="16" rx="1" />
    <path d="M8 12L3 7v10z" />
    <path d="M16 12l5-5v10z" />
  </svg>
);

export const IconZoomIn = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M12 5v14M5 12h14" />
  </svg>
);

export const IconZoomOut = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
    <path d="M5 12h14" />
  </svg>
);

export const IconFit = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <rect x="4" y="4" width="16" height="16" rx="2" />
  </svg>
);

export const IconAdvanced = () => (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 20h9M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
  </svg>
);

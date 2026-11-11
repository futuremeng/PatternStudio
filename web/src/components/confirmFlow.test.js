// Covers the confirmation flows added for destructive / state-replacing actions:
// the shared ConfirmModal and the exported-layer ("snapshot") delete in the
// layer panel. Written with createElement so it runs under the existing
// `src/**/*.test.js` vitest include pattern (no JSX).
// @vitest-environment jsdom
import React, { createElement } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "react-dom/client";
import ConfirmModal from "./ConfirmModal.jsx";
import LayerPanel from "./LayerPanel.jsx";
import { I18nProvider } from "../i18n/index.jsx";

globalThis.IS_REACT_ACT_ENVIRONMENT = true;
// React 18.3+: `act` lives on the React entry point.
const act = React.act;

let root = null;
let host = null;

function render(ui) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => { root.render(createElement(I18nProvider, null, ui)); });
  return host;
}

function click(el) {
  act(() => { el.dispatchEvent(new MouseEvent("click", { bubbles: true })); });
}

const snapshot = {
  id: 7,
  name: "Snapshot 1",
  role: "edited",
  shapeCount: 3,
  dominantColor: "#123456",
  previewSvg: '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M0 0h5v5z"/></svg>',
};

afterEach(() => {
  act(() => { root?.unmount(); });
  root = null;
  host?.remove();
  host = null;
});

describe("ConfirmModal", () => {
  it("confirms and cancels through its callbacks", () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    const el = render(createElement(ConfirmModal, {
      title: "Delete", confirmLabel: "Delete", danger: true, onConfirm, onCancel,
    }, React.createElement("p", null, "Are you sure?")));

    const footer = el.querySelectorAll(".modal-footer button");
    expect(footer).toHaveLength(2);
    expect(footer[1].className).toContain("danger");
    click(footer[1]);
    expect(onConfirm).toHaveBeenCalledTimes(1);

    click(footer[0]);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("cancels on Escape", () => {
    const onCancel = vi.fn();
    render(createElement(ConfirmModal, { title: "x", onConfirm: vi.fn(), onCancel }));
    act(() => { window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" })); });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});

describe("LayerPanel snapshot list", () => {
  const panelProps = (extra) => ({
    layers: [],
    strategyInfo: null,
    profileMs: null,
    onSaveAsLayer: () => {},
    canSaveAsLayer: false,
    layerParams: {},
    onLayerParamsChange: () => {},
    onProcess: () => {},
    onCancel: () => {},
    processBusy: false,
    processPending: false,
    processed: true,
    canvasSize: { w: 10, h: 10 },
    savedLayers: [snapshot],
    ...extra,
  });

  it("asks before dropping a snapshot, then reports it once", () => {
    const onRemoveSavedLayer = vi.fn();
    const el = render(createElement(LayerPanel, panelProps({ onRemoveSavedLayer })));

    const remove = el.querySelector(".saved-layer-remove");
    expect(remove).toBeTruthy();
    expect(el.querySelector(".confirm-modal")).toBeNull();

    click(remove);
    expect(el.querySelector(".confirm-modal")).toBeTruthy();
    expect(onRemoveSavedLayer).not.toHaveBeenCalled();

    const ok = [...el.querySelectorAll(".modal-footer button")].find(b => b.className.includes("danger"));
    click(ok);
    expect(onRemoveSavedLayer).toHaveBeenCalledTimes(1);
    expect(onRemoveSavedLayer.mock.calls[0][0]).toMatchObject({ id: snapshot.id });
    expect(el.querySelector(".confirm-modal")).toBeNull();
  });

  it("cancelling the prompt keeps the snapshot", () => {
    const onRemoveSavedLayer = vi.fn();
    const el = render(createElement(LayerPanel, panelProps({ onRemoveSavedLayer })));
    click(el.querySelector(".saved-layer-remove"));
    const cancel = [...el.querySelectorAll(".modal-footer button")].find(b => !b.className.includes("danger"));
    click(cancel);
    expect(onRemoveSavedLayer).not.toHaveBeenCalled();
    expect(el.querySelector(".confirm-modal")).toBeNull();
    expect(el.querySelectorAll(".saved-layer-card")).toHaveLength(1);
  });
});

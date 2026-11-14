import React, { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

/**
 * Mac-style contextual menus: right-click (or Ctrl+click) on interactive
 * elements shows the most common actions for that element.
 *
 *   const { menu, open, close } = useContextMenu();
 *   <div onContextMenu={e => open(e, [{ label, onSelect, danger? }])} />
 *   ...
 *   <ContextMenu menu={menu} onClose={close} />
 */
export function useContextMenu() {
  const [menu, setMenu] = useState(null);
  const open = (e, items) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY, items });
  };
  const close = () => setMenu(null);
  return { menu, open, close };
}

export default function ContextMenu({ menu, onClose }) {
  const ref = useRef(null);

  useEffect(() => {
    if (!menu) return;
    const onPointerDown = (e) => {
      if (ref.current && !ref.current.contains(e.target)) onClose();
    };
    const onKey = (e) => {
      if (e.key === "Escape") onClose();
    };
    // Capture phase: the menu must dismiss before the click reaches anything else.
    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onClose);
    window.addEventListener("resize", onClose);
    ref.current?.focus();
    return () => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onClose);
      window.removeEventListener("resize", onClose);
    };
  }, [menu, onClose]);

  if (!menu) return null;
  return createPortal(
    <div
      ref={ref}
      className="context-menu"
      style={{ left: menu.x, top: menu.y }}
      role="menu"
      tabIndex={-1}
    >
      {menu.items.map((it, i) => (
        <button
          key={i}
          role="menuitem"
          className={it.danger ? "danger" : ""}
          disabled={it.disabled}
          onClick={() => {
            onClose();
            it.onSelect();
          }}
        >
          {it.label}
        </button>
      ))}
    </div>,
    document.body
  );
}

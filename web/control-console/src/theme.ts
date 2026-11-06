import { useCallback, useEffect, useState } from "react";

export type Mode = "light" | "dark";

const KEY = "yttt-console-theme";

function stored(): Mode | null {
  try {
    const value = window.localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : null;
  } catch {
    return null;
  }
}

function system(): Mode {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Semi switches its tokens on this attribute. */
function apply(mode: Mode) {
  if (mode === "dark") {
    document.body.setAttribute("theme-mode", "dark");
  } else {
    document.body.removeAttribute("theme-mode");
  }
}

/** Applies the remembered or system theme before the first render. */
export function initTheme() {
  apply(stored() ?? system());
}

export function useTheme(): [Mode, () => void] {
  const [mode, setMode] = useState<Mode>(() => stored() ?? system());
  useEffect(() => {
    apply(mode);
  }, [mode]);
  const toggle = useCallback(() => {
    setMode((current) => {
      const next = current === "dark" ? "light" : "dark";
      try {
        window.localStorage.setItem(KEY, next);
      } catch {
        // Private windows: the choice lasts for this page only.
      }
      return next;
    });
  }, []);
  return [mode, toggle];
}

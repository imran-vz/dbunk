/**
 * Shared state hook for resizable side panels. Persists width and the
 * collapsed flag in `localStorage` under `storageKey` so a user's layout
 * choice survives across app launches.
 *
 * Width is clamped to `[min, max]` on every set so callers don't have to
 * remember to defend against an unbounded `dx` from a drag handle.
 */

import { useCallback, useEffect, useRef, useState } from "react";

interface UseResizableWidthOptions {
  /** localStorage namespace — pick something stable per panel. */
  storageKey: string;
  defaultWidth: number;
  min: number;
  max: number;
}

interface UseResizableWidthReturn {
  width: number;
  setWidth: (next: number) => void;
  collapsed: boolean;
  setCollapsed: (next: boolean) => void;
}

const readStoredNumber = (
  key: string,
  fallback: number,
  min: number,
  max: number,
) => {
  if (typeof window === "undefined") return fallback;
  const raw = window.localStorage.getItem(key);
  if (raw === null) return fallback;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) return fallback;
  return Math.max(min, Math.min(max, parsed));
};

const readStoredBool = (key: string) => {
  if (typeof window === "undefined") return false;
  return window.localStorage.getItem(key) === "1";
};

export function useResizableWidth({
  storageKey,
  defaultWidth,
  min,
  max,
}: UseResizableWidthOptions): UseResizableWidthReturn {
  const [width, setWidthState] = useState<number>(() =>
    readStoredNumber(storageKey, defaultWidth, min, max),
  );
  const [collapsed, setCollapsedState] = useState<boolean>(() =>
    readStoredBool(`${storageKey}.collapsed`),
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(storageKey, String(width));
  }, [storageKey, width]);

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(
      `${storageKey}.collapsed`,
      collapsed ? "1" : "0",
    );
  }, [storageKey, collapsed]);

  const setWidth = useCallback(
    (next: number) => {
      setWidthState(Math.max(min, Math.min(max, next)));
    },
    [min, max],
  );

  return { width, setWidth, collapsed, setCollapsed: setCollapsedState };
}

/**
 * Observe the rendered width of an element. Returns a ref to attach plus
 * the latest measured pixel width. The client-side initial value falls
 * back to `window.innerWidth` so compact shells can collapse optional
 * panels even in runtimes where element measurement is delayed.
 */
export function useContainerWidth<T extends HTMLElement>(): [
  React.RefObject<T | null>,
  number,
] {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerWidth,
  );

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const updateWidth = () => {
      const measured = el.getBoundingClientRect().width;
      setWidth(measured > 0 ? measured : window.innerWidth);
    };
    updateWidth();
    window.addEventListener("resize", updateWidth);
    if (typeof ResizeObserver === "undefined") {
      return () => window.removeEventListener("resize", updateWidth);
    }
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        setWidth(entry.contentRect.width || window.innerWidth);
      }
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateWidth);
    };
  }, []);

  return [ref, width];
}

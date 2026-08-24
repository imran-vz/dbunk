/* oxlint-disable anti-slop/no-runtime-typeof -- Browser and ResizeObserver globals are optional runtime capabilities in this shared hook. */
/**
 * Observe the rendered width of an element. Returns a ref to attach plus
 * the latest measured pixel width. The client-side initial value falls
 * back to `window.innerWidth` so compact shells can collapse optional
 * panels even in runtimes where element measurement is delayed.
 */

import { useEffect, useRef, useState } from "react";

// Round measured widths to this granularity so sub-pixel ResizeObserver
// noise and 1-2px jitter don't trigger downstream re-renders on every
// drag frame. Consumers only switch behaviour at breakpoint boundaries
// (hundreds of pixels), so a 16-px bucket is plenty.
const WIDTH_BUCKET_PX = 16;

function bucketWidth(raw: number): number {
  if (raw <= 0) return 0;
  return Math.round(raw / WIDTH_BUCKET_PX) * WIDTH_BUCKET_PX;
}

export function useContainerWidth<T extends HTMLElement>(): [
  React.RefObject<T | null>,
  number,
] {
  const ref = useRef<T | null>(null);
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 0 : bucketWidth(window.innerWidth),
  );

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const apply = (raw: number) => {
      const next = bucketWidth(raw > 0 ? raw : window.innerWidth);
      setWidth((prev) => (prev === next ? prev : next));
    };
    apply(el.getBoundingClientRect().width);
    const onResize = () => apply(el.getBoundingClientRect().width);
    window.addEventListener("resize", onResize);
    if (typeof ResizeObserver === "undefined") {
      return () => window.removeEventListener("resize", onResize);
    }
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        apply(entry.contentRect.width);
      }
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return [ref, width];
}

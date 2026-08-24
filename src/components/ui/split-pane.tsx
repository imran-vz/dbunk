/**
 * SplitPane — two-region split on the shared sash spec (DESIGN-SYSTEM
 * §3.5). The primary/secondary ratio persists per storageKey; the
 * secondary pane is collapsible and may render a fallback strip when
 * collapsed (the results status strip is the one sanctioned non-0px
 * collapsed form). Double-clicking the sash equalizes the split.
 */

import { useCallback, useEffect, useRef, useState } from "react";

import { Sash } from "@/components/ui/resizer-handle";
import { cn } from "@/lib/utils";

export interface SplitPaneProps {
  /** "column" stacks primary above secondary (editor/results). */
  direction?: "column" | "row";
  storageKey: string;
  /** Primary share of the container, 0..1. */
  defaultRatio?: number;
  minPrimary: number;
  minSecondary: number;
  /** Secondary collapsed → primary takes everything. */
  collapsed?: boolean;
  onCollapse?: () => void;
  onExpand?: () => void;
  /** Dragging the secondary below this height/width snaps it closed. */
  snapThreshold?: number;
  /** Rendered in place of the secondary pane while collapsed. */
  collapsedFallback?: React.ReactNode;
  primary: React.ReactNode;
  secondary: React.ReactNode;
  ariaLabel: string;
  className?: string;
}

const readStoredRatio = (key: string, fallback: number): number => {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- SSR boundary.
  if (typeof window === "undefined") return fallback;
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return fallback;
    const parsed = Number(raw);
    return Number.isFinite(parsed) && parsed > 0 && parsed < 1
      ? parsed
      : fallback;
  } catch {
    return fallback;
  }
};

export function SplitPane({
  direction = "column",
  storageKey,
  defaultRatio = 0.6,
  minPrimary,
  minSecondary,
  collapsed = false,
  onCollapse,
  onExpand,
  snapThreshold,
  collapsedFallback,
  primary,
  secondary,
  ariaLabel,
  className,
}: SplitPaneProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [ratio, setRatioState] = useState(() =>
    readStoredRatio(`${storageKey}.ratio`, defaultRatio),
  );

  useEffect(() => {
    try {
      window.localStorage.setItem(`${storageKey}.ratio`, String(ratio));
    } catch {
      // Best-effort persistence.
    }
  }, [storageKey, ratio]);

  const isColumn = direction === "column";

  const containerSize = useCallback(() => {
    const el = containerRef.current;
    if (!el) return 0;
    return isColumn ? el.clientHeight : el.clientWidth;
  }, [isColumn]);

  /** Secondary size in px for the sash's value/min/max contract. */
  const secondarySize = () => {
    const total = containerSize();
    return total > 0 ? Math.round(total * (1 - ratio)) : 0;
  };

  const setSecondarySize = useCallback(
    (nextSecondary: number) => {
      const total = containerSize();
      if (total <= 0) return;
      const clampedSecondary = Math.max(
        minSecondary,
        Math.min(total - minPrimary, nextSecondary),
      );
      setRatioState(1 - clampedSecondary / total);
    },
    [containerSize, minPrimary, minSecondary],
  );

  const total = containerSize();
  const maxSecondary =
    total > 0 ? Math.max(minSecondary, total - minPrimary) : 10_000;

  return (
    <div
      ref={containerRef}
      data-slot="split-pane"
      className={cn(
        "flex min-h-0 min-w-0 flex-1",
        isColumn ? "flex-col" : "flex-row",
        className,
      )}
    >
      <div
        className="flex min-h-0 min-w-0 flex-col overflow-hidden"
        style={
          collapsed ? { flex: "1 1 0%" } : { flex: `${ratio} ${ratio} 0%` }
        }
      >
        {primary}
      </div>
      <Sash
        orientation={isColumn ? "horizontal" : "vertical"}
        side={isColumn ? "top" : "left"}
        value={secondarySize()}
        min={minSecondary}
        max={maxSecondary}
        collapsed={collapsed}
        snapThreshold={snapThreshold}
        onResize={setSecondarySize}
        onCollapse={onCollapse}
        onExpand={(size) => {
          onExpand?.();
          setSecondarySize(size);
        }}
        onAutoFit={() => setRatioState(0.5)}
        ariaLabel={ariaLabel}
      />
      {collapsed ? (
        (collapsedFallback ?? null)
      ) : (
        <div
          className="flex min-h-0 min-w-0 flex-col overflow-hidden"
          style={{ flex: `${1 - ratio} ${1 - ratio} 0%` }}
        >
          {secondary}
        </div>
      )}
    </div>
  );
}

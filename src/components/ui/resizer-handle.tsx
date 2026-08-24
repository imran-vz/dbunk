/**
 * Sash — the one resize handle for every panel and split in the app
 * (DESIGN-SYSTEM §3.4). 1px visual line, 8px hit target, pointer-
 * captured live drag with snap-close below a threshold, double-click
 * auto-fit, Alt+double-click collapse, and full keyboard access
 * (arrows resize 8px / 32px with Shift, Enter toggles collapse,
 * Home/End jump to min/max).
 *
 * Orientation describes the handle itself (matches the ARIA spec):
 *   "vertical"   — a vertical line splitting left/right panes; drags on
 *                  the X axis. Used with `side: "right" | "left"`.
 *   "horizontal" — a horizontal line splitting top/bottom panes; drags
 *                  on the Y axis. Used with `side: "bottom" | "top"`.
 */

import { useRef, useState } from "react";

import { cn } from "@/lib/utils";

export interface SashProps {
  /** Current size (px) of the controlled pane; the drag origin. */
  value: number;
  onResize: (next: number) => void;
  /**
   * For vertical orientation:
   *   "right" — handle on the right edge of a left-anchored pane;
   *             dragging right grows the pane.
   *   "left"  — handle on the left edge of a right-anchored pane;
   *             dragging left grows the pane.
   * For horizontal orientation:
   *   "bottom" — handle on the bottom edge of a top-anchored pane;
   *              dragging down grows the pane.
   *   "top"    — handle on the top edge of a bottom-anchored pane;
   *              dragging up grows the pane.
   */
  side?: "right" | "left" | "bottom" | "top";
  orientation?: "vertical" | "horizontal";
  min: number;
  max: number;
  /**
   * Whether the controlled pane is currently collapsed. A collapsed
   * pane's sash stays rendered at the edge so dragging outward
   * re-opens it (the "edge-drag" restore path, §3.2).
   */
  collapsed?: boolean;
  /**
   * Dragging below this size snaps the pane closed (calls
   * `onCollapse`); continuing the same drag back past it re-opens
   * (calls `onExpand`). Omit to disable snap-close.
   */
  snapThreshold?: number;
  /** Snap-close, Alt+double-click, and Enter-on-open collapse. */
  onCollapse?: () => void;
  /** Edge-drag reopen and Enter-on-collapsed expand. */
  onExpand?: (size: number) => void;
  /** Double-click. Auto-fit to content, or equalize for splits. */
  onAutoFit?: () => void;
  className?: string;
  ariaLabel: string;
}

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- interactive split-pane separator; hr cannot carry handlers */
export function Sash({
  value,
  onResize,
  side = "right",
  orientation = "vertical",
  min,
  max,
  collapsed = false,
  snapThreshold,
  onCollapse,
  onExpand,
  onAutoFit,
  className,
  ariaLabel,
}: SashProps) {
  const dragRef = useRef<{
    pos: number;
    size: number;
    collapsed: boolean;
  } | null>(null);
  const [dragging, setDragging] = useState(false);
  const isVertical = orientation === "vertical";

  const direction = isVertical
    ? side === "right"
      ? 1
      : -1
    : side === "bottom"
      ? 1
      : -1;

  const clamp = (next: number) => Math.max(min, Math.min(max, next));

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    dragRef.current = {
      pos: isVertical ? event.clientX : event.clientY,
      size: collapsed ? 0 : value,
      collapsed,
    };
    setDragging(true);
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- jsdom lacks pointer capture.
    if (typeof event.currentTarget.setPointerCapture === "function") {
      event.currentTarget.setPointerCapture(event.pointerId);
    }
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    const current = isVertical ? event.clientX : event.clientY;
    const proposed = drag.size + (current - drag.pos) * direction;

    if (snapThreshold !== undefined) {
      if (!drag.collapsed && proposed < snapThreshold) {
        // Snap closed — collapse, not 0-width limbo (§3.4).
        drag.collapsed = true;
        onCollapse?.();
        return;
      }
      if (drag.collapsed && proposed >= snapThreshold) {
        drag.collapsed = false;
        onExpand?.(clamp(proposed));
        return;
      }
      if (drag.collapsed) return;
    }
    onResize(clamp(proposed));
  };

  const stop = (event: React.PointerEvent<HTMLDivElement>) => {
    dragRef.current = null;
    setDragging(false);
    if (
      // oxlint-disable-next-line anti-slop/no-runtime-typeof -- jsdom lacks pointer capture.
      typeof event.currentTarget.hasPointerCapture === "function" &&
      event.currentTarget.hasPointerCapture(event.pointerId)
    ) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleDoubleClick = (event: React.MouseEvent<HTMLDivElement>) => {
    if (event.altKey) {
      onCollapse?.();
      return;
    }
    onAutoFit?.();
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Enter") {
      event.preventDefault();
      if (collapsed) onExpand?.(clamp(value));
      else onCollapse?.();
      return;
    }
    if (collapsed) return;
    if (event.key === "Home") {
      event.preventDefault();
      onResize(min);
      return;
    }
    if (event.key === "End") {
      event.preventDefault();
      onResize(max);
      return;
    }
    const step = event.shiftKey ? 32 : 8;
    const decKey = isVertical ? "ArrowLeft" : "ArrowUp";
    const incKey = isVertical ? "ArrowRight" : "ArrowDown";
    if (event.key === decKey) {
      event.preventDefault();
      onResize(clamp(value - step * direction));
    } else if (event.key === incKey) {
      event.preventDefault();
      onResize(clamp(value + step * direction));
    }
  };

  return (
    <div
      role="separator"
      tabIndex={0}
      data-dragging={dragging || undefined}
      aria-orientation={orientation}
      aria-label={ariaLabel}
      aria-valuenow={collapsed ? 0 : Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={stop}
      onPointerCancel={stop}
      onDoubleClick={handleDoubleClick}
      onKeyDown={handleKeyDown}
      className={cn(
        // ~250ms hover-highlight delay (anti-flicker); instant while
        // dragging or keyboard-focused (§3.4).
        "group/sash relative z-10 shrink-0 touch-none select-none bg-border-subtle transition-colors delay-0 duration-0 hover:bg-accent hover:transition-none hover:delay-[250ms] focus-visible:bg-accent focus-visible:delay-0 focus-visible:outline-none data-dragging:bg-accent data-dragging:delay-0",
        isVertical ? "w-px cursor-col-resize" : "h-px cursor-row-resize",
        className,
      )}
    >
      {/* 8px hit target extended via pseudo-content, never layout width. */}
      <span
        aria-hidden
        className={cn(
          "absolute",
          isVertical
            ? "inset-y-0 -right-1 -left-1 cursor-col-resize"
            : "inset-x-0 -top-1 -bottom-1 cursor-row-resize",
        )}
      />
    </div>
  );
}
/* oxlint-enable jsx-a11y/prefer-tag-over-role */

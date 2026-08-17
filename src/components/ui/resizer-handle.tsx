/**
 * Drag handle for resizing a split-pane. Reports proposed sizes (in px)
 * while the user drags; the caller clamps and applies. Uses pointer
 * capture so the drag survives the cursor leaving the handle's 1 px
 * visual region.
 *
 * Orientation describes the handle itself (matches the ARIA spec):
 *   "vertical"   — a vertical line splitting left/right panes; drags on
 *                  the X axis. Used with `side: "right" | "left"`.
 *   "horizontal" — a horizontal line splitting top/bottom panes; drags
 *                  on the Y axis. Used with `side: "bottom" | "top"`.
 */

import { useRef } from "react";

import { cn } from "@/lib/utils";

interface ResizerHandleProps {
  /** Current size of the pane being resized; used as the drag origin. */
  width: number;
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
  /** Min/max for `aria-valuemin` / `aria-valuemax`; matches the caller's
   *  clamp range so AT users get accurate range info. */
  min?: number;
  max?: number;
  className?: string;
  ariaLabel?: string;
}

/* oxlint-disable jsx-a11y/prefer-tag-over-role -- interactive split-pane separator; hr cannot carry handlers */
export function ResizerHandle({
  width,
  onResize,
  side = "right",
  orientation = "vertical",
  min,
  max,
  className,
  ariaLabel,
}: ResizerHandleProps) {
  const startRef = useRef<{ pos: number; size: number } | null>(null);
  const isVertical = orientation === "vertical";

  const direction = isVertical
    ? side === "right"
      ? 1
      : -1
    : side === "bottom"
      ? 1
      : -1;

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    startRef.current = {
      pos: isVertical ? event.clientX : event.clientY,
      size: width,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!startRef.current) return;
    const current = isVertical ? event.clientX : event.clientY;
    const delta = current - startRef.current.pos;
    onResize(startRef.current.size + delta * direction);
  };

  const stop = (event: React.PointerEvent<HTMLDivElement>) => {
    startRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8;
    const decKey = isVertical ? "ArrowLeft" : "ArrowUp";
    const incKey = isVertical ? "ArrowRight" : "ArrowDown";
    if (event.key === decKey) {
      event.preventDefault();
      onResize(width - step * direction);
    } else if (event.key === incKey) {
      event.preventDefault();
      onResize(width + step * direction);
    }
  };

  return (
    <div
      role="separator"
      tabIndex={0}
      aria-orientation={orientation}
      aria-label={ariaLabel ?? "Resize pane"}
      aria-valuenow={Math.round(width)}
      aria-valuemin={min}
      aria-valuemax={max}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={stop}
      onPointerCancel={stop}
      onKeyDown={handleKeyDown}
      className={cn(
        "group/resizer relative z-10 shrink-0 touch-none select-none bg-border-subtle transition-colors hover:bg-accent/60 focus-visible:bg-accent/60 focus-visible:outline-none active:bg-accent",
        isVertical ? "w-px cursor-col-resize" : "h-px cursor-row-resize",
        className,
      )}
    >
      {/* Wider hit area so users don't need pixel-perfect aim. */}
      <span
        aria-hidden
        className={cn(
          "absolute",
          isVertical
            ? "inset-y-0 -left-1.5 -right-1.5"
            : "inset-x-0 -top-1.5 -bottom-1.5",
        )}
      />
    </div>
  );
}
/* oxlint-enable jsx-a11y/prefer-tag-over-role */

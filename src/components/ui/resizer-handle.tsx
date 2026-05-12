/**
 * Vertical drag handle for resizing a side panel. Reports proposed
 * widths (in px) while the user drags; caller clamps and applies. Uses
 * pointer capture so the drag survives the cursor leaving the handle's
 * 1 px visual region.
 */

import { useRef } from "react";

import { cn } from "@/lib/utils";

interface ResizerHandleProps {
  /** Current width of the panel being resized; used as the drag origin. */
  width: number;
  onResize: (next: number) => void;
  /**
   * "right" — handle sits on the right edge of a left-anchored panel;
   *           dragging right grows the panel.
   * "left"  — handle sits on the left edge of a right-anchored panel;
   *           dragging left grows the panel.
   */
  side?: "right" | "left";
  /** Min/max for `aria-valuemin` / `aria-valuemax`; matches the caller's
   *  clamp range so AT users get accurate range info. */
  min?: number;
  max?: number;
  className?: string;
  ariaLabel?: string;
}

export function ResizerHandle({
  width,
  onResize,
  side = "right",
  min,
  max,
  className,
  ariaLabel,
}: ResizerHandleProps) {
  const startRef = useRef<{ x: number; width: number } | null>(null);

  const handlePointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    startRef.current = { x: event.clientX, width };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const handlePointerMove = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!startRef.current) return;
    const dx = event.clientX - startRef.current.x;
    const direction = side === "right" ? 1 : -1;
    onResize(startRef.current.width + dx * direction);
  };

  const stop = (event: React.PointerEvent<HTMLDivElement>) => {
    startRef.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8;
    const direction = side === "right" ? 1 : -1;
    if (event.key === "ArrowLeft") {
      event.preventDefault();
      onResize(width - step * direction);
    } else if (event.key === "ArrowRight") {
      event.preventDefault();
      onResize(width + step * direction);
    }
  };

  return (
    // biome-ignore lint/a11y/useSemanticElements: interactive split-pane separator — <hr> can't carry handlers
    <div
      role="separator"
      tabIndex={0}
      aria-orientation="vertical"
      aria-label={ariaLabel ?? "Resize panel"}
      aria-valuenow={Math.round(width)}
      aria-valuemin={min}
      aria-valuemax={max}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={stop}
      onPointerCancel={stop}
      onKeyDown={handleKeyDown}
      className={cn(
        "group/resizer relative z-10 w-px shrink-0 cursor-col-resize touch-none select-none bg-border-subtle transition-colors hover:bg-accent-green/60 focus-visible:bg-accent-green/60 focus-visible:outline-none active:bg-accent-green",
        className,
      )}
    >
      {/* Wider hit area so users don't need pixel-perfect aim. */}
      <span aria-hidden className="absolute inset-y-0 -left-1.5 -right-1.5" />
    </div>
  );
}

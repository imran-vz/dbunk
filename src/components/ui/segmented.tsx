import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export interface SegmentedOption<T extends string> {
  id: T;
  label: string;
  icon?: ReactNode;
}

/**
 * Segmented control for sub-view toggles (DESIGN-SYSTEM §4.4).
 * Sits at `--control-h` so it aligns with the other controls in a
 * toolbar/tab row; the active segment reads as a selected surface
 * step, not an accent fill.
 */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
  className,
}: {
  options: SegmentedOption<T>[];
  value: T;
  onChange: (id: T) => void;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "inline-flex h-(--control-h) items-center gap-0.5 rounded-sm border border-border-subtle bg-surface-input p-0.5",
        className,
      )}
    >
      {options.map((opt) => {
        const active = opt.id === value;
        return (
          <button
            key={opt.id}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(opt.id)}
            className={cn(
              "flex h-full items-center gap-1.5 rounded-sm px-2 text-xs font-medium transition-colors [&_svg:not([class*='size-'])]:size-3.5",
              active
                ? "bg-surface-panel-elevated text-foreground"
                : "text-text-muted hover:text-foreground",
            )}
          >
            {opt.icon}
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

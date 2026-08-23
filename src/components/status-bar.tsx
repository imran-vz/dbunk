import type * as React from "react";
import { useEffect, useRef } from "react";

import { StatusDot, type StatusTone } from "@/components/ui/status-dot";
import { cn } from "@/lib/utils";

export type StatusBarItem = {
  id: string;
  label?: string;
  value: React.ReactNode;
  tone?: StatusTone;
  align?: "left" | "right";
};

/**
 * Field-wise equality for status items. Builders produce flat items
 * whose fields are primitives (id/label/value/tone/align), so `===`
 * per field is exact. If a future builder puts a React element in
 * `value`, this degrades to reference equality for that field — items
 * then always count as changed (notify more often, never loop-prone
 * suppression).
 */
export function statusItemsEqual(
  previous: readonly StatusBarItem[] | null,
  next: readonly StatusBarItem[],
): boolean {
  if (previous === null || previous.length !== next.length) return false;
  return previous.every(
    (item, index) =>
      item.id === next[index].id &&
      item.label === next[index].label &&
      item.value === next[index].value &&
      item.tone === next[index].tone &&
      item.align === next[index].align,
  );
}

/**
 * Notify `onChange` when the status items' content actually changes.
 *
 * Panels rebuild the items array on every render, so an unguarded
 * `useEffect(..., [onChange, items])` loops: notify → parent setState →
 * re-render → fresh array → effect refires → notify → … until React
 * aborts with "Maximum update depth exceeded". The ref-guarded content
 * compare breaks that cycle while still forwarding every real change.
 */
export function useStableStatusItems(
  items: StatusBarItem[],
  onChange?: (items: StatusBarItem[]) => void,
): void {
  const lastNotified = useRef<StatusBarItem[] | null>(null);
  useEffect(() => {
    if (!onChange || statusItemsEqual(lastNotified.current, items)) return;
    lastNotified.current = items;
    onChange(items);
  }, [onChange, items]);
}

export interface StatusBarProps {
  items: StatusBarItem[];
  className?: string;
}

export function StatusBar({ items, className }: StatusBarProps) {
  const left = items.filter((item) => item.align !== "right");
  const right = items.filter((item) => item.align === "right");

  return (
    <div
      data-slot="status-bar"
      className={cn(
        "flex h-6 shrink-0 items-center justify-between gap-3 border-t border-border-subtle bg-surface-window px-3 text-2xs text-text-muted",
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-3 overflow-hidden">
        {left.map((item) => (
          <StatusBarPair key={item.id} item={item} />
        ))}
      </div>
      <div className="flex min-w-0 items-center gap-3 overflow-hidden">
        {right.map((item) => (
          <StatusBarPair key={item.id} item={item} />
        ))}
      </div>
    </div>
  );
}

function StatusBarPair({ item }: { item: StatusBarItem }) {
  return (
    <span className="flex min-w-0 items-center gap-1.5 whitespace-nowrap">
      {item.tone ? <StatusDot tone={item.tone} className="size-1.5" /> : null}
      {item.label ? (
        <span className="text-text-muted/80">{item.label}</span>
      ) : null}
      <span className="text-text-secondary">{item.value}</span>
    </span>
  );
}

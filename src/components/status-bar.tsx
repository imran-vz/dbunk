import type * as React from "react";

import { StatusDot, type StatusTone } from "@/components/ui/status-dot";
import { cn } from "@/lib/utils";

export type StatusBarItem = {
  id: string;
  label?: string;
  value: React.ReactNode;
  tone?: StatusTone;
  align?: "left" | "right";
};

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
        "flex h-6 shrink-0 items-center justify-between gap-3 border-t border-border-subtle bg-surface-window px-3 text-[0.625rem] text-text-muted",
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

import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

interface MetaStripProps {
  children: ReactNode;
  className?: string;
}

export function MetaStrip({ children, className }: MetaStripProps) {
  return (
    <div
      className={cn(
        "flex shrink-0 items-center gap-3 border-b border-border-subtle px-3 py-1.5 text-[11px] text-text-muted",
        className,
      )}
    >
      {children}
    </div>
  );
}

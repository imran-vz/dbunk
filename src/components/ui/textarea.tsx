import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Shares the `Input` field treatment (bg, border, radius 4, 13px
 * text); height is content-driven with a two-row minimum
 * (DESIGN-SYSTEM §4.2).
 */
function Textarea({ className, ...props }: React.ComponentProps<"textarea">) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        "border-border-subtle bg-surface-input text-foreground placeholder:text-text-muted focus-visible:border-primary focus-visible:ring-primary/30 aria-invalid:ring-destructive/30 aria-invalid:border-destructive resize-none rounded-sm border px-2 py-1.5 text-sm transition-colors focus-visible:ring-[1.5px] aria-invalid:ring-[1.5px] flex field-sizing-content min-h-12 w-full outline-none disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };

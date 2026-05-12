import { Input as InputPrimitive } from "@base-ui/react/input";
import type * as React from "react";

import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <InputPrimitive
      type={type}
      data-slot="input"
      className={cn(
        "border-border-subtle bg-surface-input text-foreground placeholder:text-text-muted focus-visible:border-primary focus-visible:ring-primary/30 aria-invalid:ring-destructive/30 aria-invalid:border-destructive h-8 rounded-sm border px-2.5 py-1 text-xs/relaxed transition-colors file:h-5 file:text-xs/relaxed file:font-medium focus-visible:ring-[1.5px] aria-invalid:ring-[1.5px] file:text-foreground w-full min-w-0 outline-none file:inline-flex file:border-0 file:bg-transparent disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}

export { Input };

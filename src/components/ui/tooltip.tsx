"use client";

import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";

import { Kbd } from "@/components/ui/kbd";
import { cn } from "@/lib/utils";

/**
 * A single app-wide `TooltipProvider` with a ~400 ms open delay and
 * instant reshow while any tooltip is live (DESIGN-SYSTEM §4.7).
 * Mount this once at the workspace root. Tooltips are the only
 * `title=`-replacement — the raw `title` attribute is banned.
 */
function TooltipProvider({
  delay = 400,
  closeDelay = 0,
  ...props
}: TooltipPrimitive.Provider.Props) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delay={delay}
      closeDelay={closeDelay}
      {...props}
    />
  );
}

function Tooltip({ ...props }: TooltipPrimitive.Root.Props) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

function TooltipTrigger({ ...props }: TooltipPrimitive.Trigger.Props) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent({
  className,
  side = "top",
  sideOffset = 4,
  kbd,
  children,
  ...props
}: TooltipPrimitive.Popup.Props &
  Pick<TooltipPrimitive.Positioner.Props, "side" | "sideOffset"> & {
    /** Shortcut tokens (e.g. ["mod", "k"]) shown after the label. */
    kbd?: ReadonlyArray<string>;
  }) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Positioner
        side={side}
        sideOffset={sideOffset}
        className="isolate z-50 outline-none"
      >
        <TooltipPrimitive.Popup
          data-slot="tooltip-content"
          className={cn(
            "max-w-xs overflow-hidden rounded-md border border-border-subtle bg-surface-panel-elevated text-foreground shadow-lg ring-1 ring-black/10",
            "px-2.5 py-1.5 text-2xs leading-relaxed",
            "data-[starting-style]:scale-95 data-[starting-style]:opacity-0 data-[ending-style]:scale-95 data-[ending-style]:opacity-0",
            "origin-[var(--transform-origin)] transition-[opacity,transform] duration-100",
            className,
          )}
          {...props}
        >
          {kbd ? (
            <span className="inline-flex items-center gap-1.5">
              {children}
              <Kbd keys={kbd} />
            </span>
          ) : (
            children
          )}
        </TooltipPrimitive.Popup>
      </TooltipPrimitive.Positioner>
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger };

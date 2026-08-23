import { Button as ButtonPrimitive } from "@base-ui/react/button";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/**
 * Control heights come from the density variables (`--control-h*`,
 * see styles.css) so Button, Input and SelectTrigger share one height
 * at every density (DESIGN-SYSTEM §2.3/§4.1). Icon sizing is owned by
 * the button size — callsites must not override it.
 */
const buttonVariants = cva(
  "focus-visible:border-ring focus-visible:ring-ring/40 aria-invalid:ring-destructive/30 aria-invalid:border-destructive rounded-sm border border-transparent bg-clip-padding font-medium focus-visible:ring-[1.5px] aria-invalid:ring-[1.5px] inline-flex items-center justify-center whitespace-nowrap transition-colors disabled:pointer-events-none disabled:opacity-50 [&_svg]:pointer-events-none shrink-0 [&_svg]:shrink-0 outline-none group/button select-none",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground hover:bg-accent-hover",
        outline:
          "border-border-subtle bg-surface-panel text-foreground hover:border-border-strong hover:bg-surface-panel-elevated aria-expanded:bg-surface-panel-elevated",
        secondary:
          "bg-surface-panel-elevated text-foreground hover:bg-surface-row-hover aria-expanded:bg-surface-row-hover",
        ghost:
          "text-text-secondary hover:bg-surface-panel hover:text-foreground aria-expanded:bg-surface-panel aria-expanded:text-foreground",
        destructive:
          "bg-danger/15 text-danger hover:bg-danger/25 focus-visible:ring-danger/30",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default:
          "h-(--control-h) gap-1.5 px-2 text-sm has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-4",
        xs: "h-(--control-h-sm) gap-1 px-1.5 text-2xs has-data-[icon=inline-end]:pr-1 has-data-[icon=inline-start]:pl-1 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-(--control-h-sm) gap-1.5 px-2 text-xs has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-(--control-h-lg) gap-1.5 px-2.5 text-sm has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-4",
        icon: "size-auto h-(--control-h) w-(--control-h) [&_svg:not([class*='size-'])]:size-4",
        "icon-xs":
          "size-auto h-(--control-h-sm) w-(--control-h-sm) [&_svg:not([class*='size-'])]:size-3",
        "icon-sm":
          "size-auto h-(--control-h-sm) w-(--control-h-sm) [&_svg:not([class*='size-'])]:size-3.5",
        "icon-lg":
          "size-auto h-(--control-h-lg) w-(--control-h-lg) [&_svg:not([class*='size-'])]:size-4",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      data-size={size ?? "default"}
      data-variant={variant ?? "default"}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };

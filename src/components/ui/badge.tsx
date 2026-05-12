import { mergeProps } from "@base-ui/react/merge-props";
import { useRender } from "@base-ui/react/use-render";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "h-[1.125rem] gap-1 rounded-sm border border-transparent px-1.5 py-0 text-[0.625rem] font-medium transition-colors has-data-[icon=inline-end]:pr-1 has-data-[icon=inline-start]:pl-1 [&>svg]:size-2.5! inline-flex items-center justify-center w-fit whitespace-nowrap shrink-0 [&>svg]:pointer-events-none focus-visible:border-ring focus-visible:ring-ring/40 focus-visible:ring-[1.5px] aria-invalid:ring-destructive/30 aria-invalid:border-destructive overflow-hidden group/badge",
  {
    variants: {
      variant: {
        default: "bg-primary text-primary-foreground [a]:hover:bg-primary/80",
        secondary:
          "bg-surface-panel-elevated text-text-secondary [a]:hover:bg-surface-row-hover",
        destructive: "bg-danger/15 text-danger [a]:hover:bg-danger/25",
        outline:
          "border-border-subtle text-text-secondary bg-surface-panel [a]:hover:bg-surface-panel-elevated",
        success: "bg-accent-green-subdued text-accent-green-hover",
        warning: "bg-warning/15 text-warning",
        info: "bg-info/15 text-info",
        ghost: "hover:bg-surface-panel hover:text-foreground",
        link: "text-primary underline-offset-4 hover:underline",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function Badge({
  className,
  variant = "default",
  render,
  ...props
}: useRender.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return useRender({
    defaultTagName: "span",
    props: mergeProps<"span">(
      {
        className: cn(badgeVariants({ className, variant })),
      },
      props,
    ),
    render,
    state: {
      slot: "badge",
      variant,
    },
  });
}

export { Badge, badgeVariants };

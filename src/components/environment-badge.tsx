import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { ENVIRONMENT_META, resolveSafetyPolicy } from "@/lib/safety-policy";
import type { ConnectionEnvironment } from "@/lib/store";
import { cn } from "@/lib/utils";

const ENVIRONMENT_BADGE_VARIANT = {
  development: "outline",
  test: "info",
  staging: "warning",
  production: "destructive",
} as const satisfies Record<
  ConnectionEnvironment,
  React.ComponentProps<typeof Badge>["variant"]
>;

export function EnvironmentBadge({
  environment,
  className,
  short = false,
}: {
  environment: ConnectionEnvironment | undefined;
  className?: string;
  short?: boolean;
}) {
  const resolvedEnvironment = resolveSafetyPolicy({ environment }).environment;
  if (resolvedEnvironment === "development") return null;

  const meta = ENVIRONMENT_META[resolvedEnvironment];
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Badge
            variant={ENVIRONMENT_BADGE_VARIANT[resolvedEnvironment]}
            className={cn("uppercase tracking-wide", className)}
          />
        }
      >
        {short ? meta.shortLabel : meta.label}
      </TooltipTrigger>
      <TooltipContent>{meta.description}</TooltipContent>
    </Tooltip>
  );
}

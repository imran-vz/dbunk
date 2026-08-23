import type * as React from "react";

import { cn } from "@/lib/utils";

export type StatusTone = "healthy" | "warning" | "danger" | "neutral" | "info";

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const toneClass: Record<StatusTone, string> = {
  healthy: "bg-success shadow-[0_0_10px_rgba(143,221,76,0.55)]",
  warning: "bg-warning shadow-[0_0_10px_rgba(245,184,75,0.45)]",
  danger: "bg-danger shadow-[0_0_10px_rgba(239,107,107,0.45)]",
  info: "bg-info shadow-[0_0_10px_rgba(103,183,255,0.45)]",
  neutral: "bg-text-disabled",
};

export function StatusDot({
  tone = "healthy",
  className,
  ...props
}: { tone?: StatusTone } & React.HTMLAttributes<HTMLSpanElement>) {
  return (
    <span
      data-slot="status-dot"
      className={cn(
        "inline-block size-2 shrink-0 rounded-full",
        toneClass[tone],
        className,
      )}
      {...props}
    />
  );
}

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const toneLabel: Record<StatusTone, string> = {
  healthy: "Healthy",
  warning: "Warning",
  danger: "Error",
  info: "Info",
  neutral: "Disconnected",
};

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const toneText: Record<StatusTone, string> = {
  healthy: "text-accent-hover",
  warning: "text-warning",
  danger: "text-danger",
  info: "text-info",
  neutral: "text-text-muted",
};

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const toneBg: Record<StatusTone, string> = {
  healthy: "bg-accent-subdued/70",
  warning: "bg-warning/15",
  danger: "bg-danger/15",
  info: "bg-info/15",
  neutral: "bg-surface-panel-elevated",
};

export function HealthPill({
  tone = "healthy",
  label,
  className,
}: {
  tone?: StatusTone;
  label?: string;
  className?: string;
}) {
  return (
    <span
      data-slot="health-pill"
      className={cn(
        "inline-flex h-5 items-center gap-1.5 rounded-full px-2 text-2xs font-medium",
        toneBg[tone],
        toneText[tone],
        className,
      )}
    >
      <StatusDot tone={tone} className="size-1.5" />
      {label ?? toneLabel[tone]}
    </span>
  );
}

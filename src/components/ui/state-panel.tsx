/**
 * Shared empty / loading / error state primitives (DESIGN-SYSTEM
 * §4.11). All app regions with a fetch lifecycle must use these —
 * local reimplementations are banned:
 *
 *   if (loading) return <LoadingState label="Loading sessions…" />;
 *   if (error)   return <ErrorState message={error} onRetry={retry} />;
 *   if (rows.length === 0) return <EmptyState title="No sessions" />;
 *
 * Loading is a 2px indeterminate accent bar at the region's top edge —
 * never a centered spinner (spinners are reserved for ≥500ms
 * button-level operations). Empty is one muted line plus at most one
 * action. Error is an inline danger banner with expandable mono
 * details.
 */

import { IconRefresh } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface BaseShellProps {
  className?: string;
}

/**
 * 2px indeterminate accent bar for a region's top edge. Position it
 * inside a `relative` container when overlaying existing (dimmed)
 * content.
 */
export function LoadingBar({ className }: BaseShellProps) {
  return (
    <div
      aria-hidden
      className={cn(
        "pointer-events-none absolute inset-x-0 top-0 h-0.5 overflow-hidden",
        className,
      )}
    >
      <div className="h-full w-1/3 animate-loading-bar bg-primary" />
    </div>
  );
}

export function LoadingState({
  label = "Loading…",
  className,
}: BaseShellProps & { label?: string }) {
  return (
    <output
      aria-live="polite"
      className={cn("relative block h-full min-h-4 w-full", className)}
    >
      <LoadingBar />
      <span className="sr-only">{label}</span>
    </output>
  );
}

export function ErrorState({
  message,
  details,
  onRetry,
  className,
}: BaseShellProps & {
  message: string;
  /** Raw driver/server output, shown behind a disclosure in mono. */
  details?: string;
  onRetry?: () => void;
}) {
  return (
    <div
      role="alert"
      className={cn(
        "flex flex-col gap-2 border-l-2 border-danger bg-danger/10 px-3 py-2",
        className,
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 text-sm text-foreground">{message}</div>
        {onRetry ? (
          <Button size="sm" variant="outline" onClick={onRetry}>
            <IconRefresh /> Retry
          </Button>
        ) : null}
      </div>
      {details ? (
        <details className="min-w-0">
          <summary className="cursor-default text-xs text-text-muted select-none">
            Details
          </summary>
          <pre className="mt-1 overflow-x-auto font-mono text-xs whitespace-pre-wrap text-text-secondary">
            {details}
          </pre>
        </details>
      ) : null}
    </div>
  );
}

export function EmptyState({
  title,
  description,
  action,
  className,
}: BaseShellProps & {
  title: string;
  description?: string;
  action?: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex h-full flex-col items-center justify-center gap-2 p-6 text-center",
        className,
      )}
    >
      <div className="max-w-md text-sm text-text-muted">
        {title}
        {description ? <> — {description}</> : null}
      </div>
      {action}
    </div>
  );
}

export function Skeleton({
  className,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  return (
    <div
      className={cn(
        "animate-pulse rounded-sm bg-surface-panel-elevated/60",
        className,
      )}
      {...props}
    />
  );
}

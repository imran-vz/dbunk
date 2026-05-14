/**
 * Reusable empty / loading / error state shells. All app panels with
 * a fetch lifecycle should pick one of these instead of hand-rolling
 * a "Loading…" string, so the visual language stays consistent.
 *
 * Each variant lives in its own component so callers spell out the
 * state at the call site rather than juggling a `kind` prop:
 *
 *   if (loading) return <LoadingState label="Loading sessions…" />;
 *   if (error)   return <ErrorState message={error} onRetry={retry} />;
 *   if (rows.length === 0) return <EmptyState title="No sessions" />;
 */

import {
  IconAlertTriangle,
  IconInbox,
  IconLoader2,
  IconRefresh,
} from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface BaseShellProps {
  className?: string;
}

export function LoadingState({
  label = "Loading…",
  className,
}: BaseShellProps & { label?: string }) {
  return (
    <output
      aria-live="polite"
      className={cn(
        "flex h-full flex-col items-center justify-center gap-2 p-6 text-xs text-text-muted",
        className,
      )}
    >
      <IconLoader2 className="size-5 animate-spin" />
      <span>{label}</span>
    </output>
  );
}

export function ErrorState({
  message,
  onRetry,
  className,
}: BaseShellProps & { message: string; onRetry?: () => void }) {
  return (
    <div
      role="alert"
      className={cn(
        "flex h-full flex-col items-center justify-center gap-3 p-6 text-center",
        className,
      )}
    >
      <div className="flex size-9 items-center justify-center rounded-full bg-danger/10 text-danger">
        <IconAlertTriangle className="size-5" />
      </div>
      <div className="max-w-md whitespace-pre-wrap font-mono text-xs text-danger">
        {message}
      </div>
      {onRetry ? (
        <Button size="sm" variant="outline" onClick={onRetry}>
          <IconRefresh className="size-3.5" /> Retry
        </Button>
      ) : null}
    </div>
  );
}

export function EmptyState({
  title,
  description,
  icon,
  action,
  className,
}: BaseShellProps & {
  title: string;
  description?: string;
  icon?: React.ReactNode;
  action?: React.ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex h-full flex-col items-center justify-center gap-2 p-6 text-center",
        className,
      )}
    >
      <div className="flex size-9 items-center justify-center rounded-full bg-surface-panel-elevated text-text-muted">
        {icon ?? <IconInbox className="size-5" />}
      </div>
      <div className="text-sm font-semibold text-foreground">{title}</div>
      {description ? (
        <p className="max-w-md text-xs text-text-muted">{description}</p>
      ) : null}
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
        "animate-pulse rounded-md bg-surface-panel-elevated/60",
        className,
      )}
      {...props}
    />
  );
}

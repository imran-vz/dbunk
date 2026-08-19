import {
  IconAlertTriangle,
  IconCheck,
  IconLock,
  IconX,
} from "@tabler/icons-react";
import type * as React from "react";

import { Button } from "@/components/ui/button";
import type { EditOutcome, TableEditsCommitStatus } from "@/lib/store";

interface TableStatusBannersProps {
  errorMessage: string | null;
  showReadOnlyBanner: boolean;
  readOnlyCopy?: string;
  commitStatus: TableEditsCommitStatus | undefined;
  lastOutcome: EditOutcome | null;
  onRetryLoad: () => void;
  onDismissOutcome: () => void;
}

export function TableStatusBanners({
  errorMessage,
  showReadOnlyBanner,
  readOnlyCopy,
  commitStatus,
  lastOutcome,
  onRetryLoad,
  onDismissOutcome,
}: TableStatusBannersProps) {
  return (
    <>
      {errorMessage ? (
        <Banner
          testId="table-error"
          tone="danger"
          as="div"
          role="alert"
          icon={<IconAlertTriangle className="size-4" />}
          message={<>Failed to load rows: {errorMessage}</>}
          action={
            <Button variant="ghost" size="sm" onClick={onRetryLoad}>
              Retry
            </Button>
          }
        />
      ) : null}

      {showReadOnlyBanner ? (
        <Banner
          testId="table-readonly-banner"
          tone="warning"
          as="output"
          icon={<IconLock className="size-4" />}
          message={
            <>
              {readOnlyCopy ??
                "This table has no primary key or non-null unique index — it is read-only. Add a unique constraint to enable editing."}
            </>
          }
        />
      ) : null}

      {commitStatus?.state === "queued" ? (
        <Banner
          testId="table-commit-queued"
          tone="warning"
          as="output"
          icon={<IconLock className="size-4" />}
          message={
            <>
              Queued — applying {commitStatus.mutationIds.length} mutation
              {commitStatus.mutationIds.length === 1 ? "" : "s"} in the
              background. Refreshing when complete.
            </>
          }
        />
      ) : null}

      {lastOutcome?.kind === "completed" &&
      lastOutcome.rowsAffected !== undefined ? (
        <Banner
          testId="table-commit-success"
          tone="success"
          as="output"
          icon={<IconCheck className="size-4" />}
          message={
            <>
              Saved {lastOutcome.rowsAffected} row
              {lastOutcome.rowsAffected === 1 ? "" : "s"} in{" "}
              {lastOutcome.runtimeMs} ms.
            </>
          }
          action={
            <Button variant="ghost" size="sm" onClick={onDismissOutcome}>
              Dismiss
            </Button>
          }
        />
      ) : null}

      {lastOutcome?.kind === "failed" ? (
        <Banner
          testId="table-commit-error"
          tone="danger"
          as="div"
          role="alert"
          icon={<IconX className="size-4" />}
          message={<>Failed to save: {lastOutcome.reason}</>}
          action={
            <Button variant="ghost" size="sm" onClick={onDismissOutcome}>
              Dismiss
            </Button>
          }
        />
      ) : null}

      {lastOutcome?.kind === "timeout" ? (
        <Banner
          testId="table-commit-timeout"
          tone="warning"
          as="div"
          role="alert"
          icon={<IconAlertTriangle className="size-4" />}
          message={
            <>
              Mutation did not complete in time. Check system.mutations for{" "}
              {lastOutcome.remaining.length} remaining.
            </>
          }
          action={
            <Button variant="ghost" size="sm" onClick={onDismissOutcome}>
              Dismiss
            </Button>
          }
        />
      ) : null}
    </>
  );
}

type BannerTone = "danger" | "warning" | "success";

interface BannerProps {
  testId: string;
  tone: BannerTone;
  as: "output" | "div";
  role?: "alert";
  icon: React.ReactNode;
  message: React.ReactNode;
  action?: React.ReactNode;
}

// oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
const TONE_CLASS: Record<BannerTone, string> = {
  danger: "border-danger/40 bg-danger/10 text-danger",
  warning: "border-warning/40 bg-warning/10 text-warning",
  success: "border-accent/40 bg-accent/10 text-accent-hover",
};

function Banner({
  testId,
  tone,
  as,
  role,
  icon,
  message,
  action,
}: BannerProps) {
  const className = `flex items-center gap-2 border-b px-3 py-1.5 text-xs ${TONE_CLASS[tone]}`;
  const actionWrapped = action ? (
    <span className="ml-auto">{action}</span>
  ) : null;
  if (as === "output") {
    return (
      <output data-testid={testId} className={className}>
        {icon}
        <span>{message}</span>
        {actionWrapped}
      </output>
    );
  }
  return (
    <div data-testid={testId} role={role} className={className}>
      {icon}
      <span>{message}</span>
      {actionWrapped}
    </div>
  );
}

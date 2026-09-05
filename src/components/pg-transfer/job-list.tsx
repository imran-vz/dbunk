import {
  IconAlertTriangle,
  IconCheck,
  IconClock,
  IconRefresh,
} from "@tabler/icons-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  decodePgTransferError,
  formatPgTransferError,
  isPgTransferJobActive,
  type PgTransferJob,
} from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";

const PHASE_LABEL = {
  preparing: "Preparing",
  running: "Running",
  cancelling: "Cancellation requested",
  finalizing: "Finalizing",
  completed: "Completed",
  cancelled: "Cancelled",
  failed: "Failed",
  outcomeUnknown: "Outcome unknown",
} satisfies Record<PgTransferJob["phase"], string>;

export function formatTransferBytes(bytes: number | null) {
  if (bytes === null) return "Unknown";
  if (bytes < 1024) return `${bytes} B`;
  const divisor = bytes < 1_048_576 ? 1024 : 1_048_576;
  return `${(bytes / divisor).toFixed(1)} ${divisor === 1024 ? "KB" : "MB"}`;
}

function elapsed(job: PgTransferJob) {
  const milliseconds =
    (job.finishedAt ? Date.parse(job.finishedAt) : Date.now()) -
    Date.parse(job.startedAt);
  const seconds = Math.max(0, Math.floor(milliseconds / 1000));
  return Number.isFinite(seconds)
    ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
    : "Unknown";
}

function progressCopy(job: PgTransferJob) {
  if (job.direction === "import") {
    if (job.phase === "completed") {
      return job.rowsCommitted === null
        ? "Import committed."
        : `${job.rowsCommitted.toLocaleString()} rows committed.`;
    }
    if (job.phase === "outcomeUnknown") {
      return "Rows may be committed. Inspect the target before importing again.";
    }
    if (job.phase === "cancelling") {
      return "Waiting for rollback or commit confirmation. Do not retry yet.";
    }
    if (job.phase === "finalizing") {
      return "Committing. Waiting for database acknowledgement.";
    }
    if (job.phase === "cancelled") {
      return "Import cancelled. No rows committed.";
    }
    if (job.phase === "failed") {
      return "Import failed. No rows committed.";
    }
    if (job.phase === "preparing") {
      return "Preparing the append-only transaction.";
    }
    if (job.rowsProcessed !== null) {
      return `${job.rowsProcessed.toLocaleString()} rows sent. Commit pending.`;
    }
    return "Preparing the append-only transaction.";
  }
  if (job.phase === "completed") return "Complete CSV published.";
  if (job.phase === "cancelling") {
    return "Waiting for the stream to stop and its partial file to be removed.";
  }
  if (job.phase === "finalizing") {
    return "Syncing and publishing the complete file. Cancellation is unavailable.";
  }
  if (job.phase === "cancelled") return "Export cancelled. No file published.";
  if (job.phase === "failed") return "Export failed. No file published.";
  if (job.phase === "preparing") return "Preparing the read-only export.";
  return "File is not published yet. Total size and row count are unknown.";
}

export function PgTransferJobDetails({ job }: { job: PgTransferJob }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const active = isPgTransferJobActive(job);
  const canCancel =
    active && job.phase !== "cancelling" && job.phase !== "finalizing";

  async function action() {
    setBusy(true);
    setError(null);
    try {
      if (active) await pgTransferObserver.cancel(job.jobId);
      else await pgTransferObserver.release(job.jobId);
    } catch (cause) {
      setError(formatPgTransferError(decodePgTransferError(cause)));
    } finally {
      setBusy(false);
    }
  }

  const percent =
    job.direction === "import" &&
    job.totalBytes !== null &&
    job.totalBytes > 0 &&
    job.bytesProcessed <= job.totalBytes
      ? Math.round((job.bytesProcessed / job.totalBytes) * 100)
      : null;
  const warning = job.phase === "failed" || job.phase === "outcomeUnknown";

  return (
    <section
      className="min-w-0 p-4 text-xs lg:p-6"
      aria-label="Transfer details"
    >
      <h2 className="mb-3 truncate text-sm font-semibold">
        {job.direction === "import" ? "Import" : "Export"} · {job.fileName}
      </h2>
      <div
        className={`mb-3 flex items-center gap-1.5 ${warning ? "text-warning" : "text-foreground"}`}
      >
        {warning ? (
          <IconAlertTriangle className="size-4" />
        ) : job.phase === "completed" ? (
          <IconCheck className="size-4 text-success" />
        ) : (
          <IconClock className="size-4" />
        )}
        <span>{PHASE_LABEL[job.phase]}</span>
      </div>
      <dl>
        {[
          ["Table", `${job.schema}.${job.table}`],
          ["Elapsed", elapsed(job)],
          [
            job.direction === "import" ? "Source read" : "Written",
            formatTransferBytes(job.bytesProcessed),
          ],
          [
            "Source size",
            job.direction === "import"
              ? formatTransferBytes(job.totalBytes)
              : "Unknown",
          ],
        ].map(([name, value]) => (
          <div
            key={name}
            className="flex justify-between gap-4 border-b border-border-subtle py-2"
          >
            <dt className="text-text-muted">{name}</dt>
            <dd className="break-all text-right font-mono">{value}</dd>
          </div>
        ))}
      </dl>
      {percent !== null && active ? (
        <progress
          className="mt-3 h-1 w-full overflow-hidden bg-surface-panel accent-primary"
          max={100}
          value={percent}
          aria-label="Source bytes read"
        />
      ) : null}
      <p className={`mt-3 ${warning ? "text-warning" : "text-text-secondary"}`}>
        {progressCopy(job)}
      </p>
      {job.failure ? (
        <p role="alert" className="mt-3 text-danger">
          {formatPgTransferError(job.failure)}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="mt-3 text-danger">
          {error}
        </p>
      ) : null}
      {canCancel || !active ? (
        <div className="mt-4 flex justify-end">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void action()}
          >
            {canCancel ? "Cancel transfer" : "Dismiss"}
          </Button>
        </div>
      ) : null}
    </section>
  );
}

export function PgTransferJobList({
  jobs,
  selectedId,
  onSelect,
}: {
  jobs: PgTransferJob[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  return (
    <section
      className="border-t border-border-subtle"
      aria-label="Recent transfers"
    >
      <div className="flex h-(--h-toolbar) items-center gap-2 border-b border-border-subtle bg-surface-window px-3">
        <h2 className="text-xs font-semibold">Recent transfers</h2>
        <span className="text-2xs text-text-muted">
          This session · up to 1 hour
        </span>
        <Button
          className="ml-auto"
          variant="ghost"
          size="icon-sm"
          aria-label="Refresh transfers"
          onClick={() => void pgTransferObserver.refresh()}
        >
          <IconRefresh />
        </Button>
      </div>
      {jobs.length ? (
        <div className="overflow-x-auto">
          <table className="w-full min-w-160 text-left text-xs">
            <thead className="bg-surface-sidebar text-2xs text-text-muted">
              <tr>
                {[
                  "File",
                  "Direction",
                  "Table",
                  "State",
                  "Processed",
                  "Started",
                ].map((title) => (
                  <th className="px-3 py-1 font-normal" key={title}>
                    {title}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {jobs.map((job) => (
                <tr
                  key={job.jobId}
                  className={
                    selectedId === job.jobId ? "bg-surface-panel" : undefined
                  }
                >
                  <td className="border-b border-border-subtle px-3 py-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      className="max-w-64 justify-start truncate"
                      onClick={() => onSelect(job.jobId)}
                    >
                      {job.fileName}
                    </Button>
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1 capitalize">
                    {job.direction}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1 font-mono">
                    {job.schema}.{job.table}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1">
                    {PHASE_LABEL[job.phase]}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1 font-mono">
                    {formatTransferBytes(job.bytesProcessed)}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1 font-mono">
                    {new Date(job.startedAt).toLocaleTimeString([], {
                      hour: "2-digit",
                      minute: "2-digit",
                    })}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="p-4 text-xs text-text-muted">
          No transfers in this session.
        </p>
      )}
    </section>
  );
}

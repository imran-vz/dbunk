import { IconCheck, IconClock, IconRefresh } from "@tabler/icons-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  decodePgToolError,
  formatPgToolError,
  isPgToolJobActive,
  type PgToolJob,
} from "@/lib/pg-tool-jobs/client";
import { pgToolObserver } from "@/lib/pg-tool-jobs/observer";

const PHASE_LABEL = {
  queued: "Queued",
  preflight: "Checking tools",
  running: "Running",
  finalizing: "Finishing",
  completed: "Completed",
  cancelling: "Cancelling",
  cancelled: "Cancelled",
  failed: "Failed",
} satisfies Record<PgToolJob["phase"], string>;
export function formatJobBytes(bytes: number | null) {
  if (bytes === null) return "Unavailable";
  if (bytes < 1024) return `${bytes} B`;
  return `${(bytes / (bytes < 1048576 ? 1024 : 1048576)).toFixed(1)} ${bytes < 1048576 ? "KB" : "MB"}`;
}
function elapsed(job: PgToolJob) {
  const seconds = Math.max(
    0,
    Math.floor(
      ((job.finishedAt ? Date.parse(job.finishedAt) : Date.now()) -
        Date.parse(job.startedAt)) /
        1000,
    ),
  );
  return Number.isFinite(seconds)
    ? `${Math.floor(seconds / 60)}m ${seconds % 60}s`
    : "Unavailable";
}
export function PgToolJobDetails({ job }: { job: PgToolJob }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function action() {
    setBusy(true);
    setError(null);
    try {
      if (isPgToolJobActive(job)) await pgToolObserver.cancel(job.jobId);
      else await pgToolObserver.release(job.jobId);
    } catch (cause) {
      setError(formatPgToolError(decodePgToolError(cause)));
    } finally {
      setBusy(false);
    }
  }
  const canCancel =
    isPgToolJobActive(job) &&
    job.phase !== "finalizing" &&
    job.phase !== "cancelling";
  return (
    <section
      className="min-w-0 p-4 text-xs"
      aria-label={`${job.kind} job details`}
    >
      <h2 className="mb-3 truncate text-sm font-semibold">
        {job.kind === "backup" ? "Backup" : "Restore"} · {job.fileName}
      </h2>
      <div className="mb-3 flex items-center gap-1.5 text-foreground">
        {job.phase === "completed" ? (
          <IconCheck className="size-4 text-success" />
        ) : (
          <IconClock className="size-4" />
        )}
        <span>{PHASE_LABEL[job.phase]}</span>
      </div>
      {job.kind === "restore" && job.phase === "running" ? (
        <p className="mb-3 text-text-secondary">
          Restoring archive. Progress percentage is unavailable.
        </p>
      ) : null}
      {job.phase === "cancelling" ? (
        <p className="mb-3 text-text-secondary">
          Waiting for the tool to stop. A restore that already committed may
          still complete.
        </p>
      ) : null}
      {job.phase === "finalizing" ? (
        <p className="mb-3 text-text-secondary">
          Finalizing the result. Cancellation is no longer available.
        </p>
      ) : null}
      <dl>
        {[
          ["Elapsed", elapsed(job)],
          [
            job.kind === "restore" ? "Source size" : "Written",
            formatJobBytes(
              job.kind === "restore" ? job.totalBytes : job.bytesProcessed,
            ),
          ],
          ["Tool", job.toolVersion ?? "Unavailable"],
        ].map(([name, value]) => (
          <div
            key={name}
            className="flex justify-between gap-4 border-b border-border-subtle py-2"
          >
            <dt className="text-text-muted">{name}</dt>
            <dd className="font-mono">{value}</dd>
          </div>
        ))}
      </dl>
      {job.failure ? (
        <p role="alert" className="mt-3 text-danger">
          {formatPgToolError(job.failure)}
        </p>
      ) : null}
      {error ? (
        <p role="alert" className="mt-3 text-danger">
          {error}
        </p>
      ) : null}
      {canCancel || !isPgToolJobActive(job) ? (
        <div className="mt-4 flex justify-end">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => void action()}
          >
            {canCancel ? "Cancel job" : "Dismiss"}
          </Button>
        </div>
      ) : null}
      {isPgToolJobActive(job) ? (
        <p className="mt-3 text-2xs text-text-muted">
          Cancellation cannot undo an already committed restore.
        </p>
      ) : null}
    </section>
  );
}
export function PgToolJobList({
  jobs,
  selectedId,
  onSelect,
}: {
  jobs: PgToolJob[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}) {
  return (
    <section
      className="border-t border-border-subtle"
      aria-label="Recent backup and restore jobs"
    >
      <div className="flex h-(--h-toolbar) items-center gap-2 border-b border-border-subtle bg-surface-window px-3">
        <h2 className="text-xs font-semibold">Recent jobs</h2>
        <span className="text-2xs text-text-muted">
          This session · up to 1 hour
        </span>
        <Button
          className="ml-auto"
          variant="ghost"
          size="icon-sm"
          aria-label="Refresh jobs"
          onClick={() => void pgToolObserver.refresh()}
        >
          <IconRefresh />
        </Button>
      </div>
      {jobs.length ? (
        <div className="overflow-x-auto">
          <table className="w-full text-left text-xs">
            <thead className="bg-surface-sidebar text-2xs text-text-muted">
              <tr>
                {["File", "Operation", "Status", "Elapsed"].map((title) => (
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
                      onClick={() => onSelect(job.jobId)}
                    >
                      {job.fileName}
                    </Button>
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1">
                    {job.kind}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1">
                    {PHASE_LABEL[job.phase]}
                  </td>
                  <td className="border-b border-border-subtle px-3 py-1 font-mono">
                    {elapsed(job)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <p className="p-4 text-xs text-text-muted">No jobs in this session.</p>
      )}
    </section>
  );
}

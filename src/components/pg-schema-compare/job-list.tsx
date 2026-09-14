import { IconRefresh } from "@tabler/icons-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  decodeSchemaCompareFailure,
  isSchemaCompareActive,
} from "@/lib/pg-schema-compare/failure";
import type { SchemaCompareObserver } from "@/lib/pg-schema-compare/observer";
import {
  formatSchemaCompareFailure,
  PHASE_LABEL,
  PHASE_TONE,
} from "@/lib/pg-schema-compare/presentation";
import type { SchemaCompareStatus } from "@/lib/pg-schema-compare/protocol";
import type { Connection } from "@/lib/store";
import { cn } from "@/lib/utils";

import { TONE_CLASS } from "./coverage";
import { EndpointIdentity } from "./endpoint-form";

function JobRow({
  job,
  selected,
  connections,
  observer,
  onSelect,
  onDismissed,
}: {
  job: SchemaCompareStatus;
  selected: boolean;
  connections: Connection[];
  observer: SchemaCompareObserver;
  onSelect: () => void;
  onDismissed: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = isSchemaCompareActive(job);
  async function act() {
    setBusy(true);
    setError(null);
    try {
      if (active) {
        await observer.cancel(job.jobId);
      } else {
        await observer.release(job.jobId);
        onDismissed();
      }
    } catch (cause) {
      setError(formatSchemaCompareFailure(decodeSchemaCompareFailure(cause)));
    } finally {
      setBusy(false);
    }
  }
  return (
    <li
      className={cn(
        "flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border-subtle px-(--pad-panel) py-1 text-xs",
        selected && "bg-surface-panel",
      )}
    >
      <button
        type="button"
        aria-pressed={selected}
        onClick={onSelect}
        className={cn(
          "flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-0.5 text-left outline-none focus-visible:ring-1 focus-visible:ring-accent",
          selected && "text-accent",
        )}
      >
        <span
          className={cn("w-32 shrink-0", TONE_CLASS[PHASE_TONE[job.phase]])}
        >
          {PHASE_LABEL[job.phase]}
        </span>
        <EndpointIdentity endpoint={job.source} connections={connections} />
        <span aria-hidden="true" className="text-text-muted">
          →
        </span>
        <EndpointIdentity endpoint={job.target} connections={connections} />
      </button>
      <Button
        variant="outline"
        size="sm"
        disabled={busy || job.phase === "cancelling"}
        onClick={() => void act()}
      >
        {active
          ? job.phase === "cancelling"
            ? "Cancelling…"
            : "Cancel"
          : "Dismiss"}
      </Button>
      {error ? (
        <span role="alert" className="basis-full text-danger">
          {error}
        </span>
      ) : null}
    </li>
  );
}

/** The backend's session records, each by its own endpoint identity and phase. */
export function SchemaCompareJobList({
  jobs,
  selectedId,
  connections,
  observer,
  onSelect,
  onDismissed,
}: {
  jobs: SchemaCompareStatus[];
  selectedId: string | null;
  connections: Connection[];
  observer: SchemaCompareObserver;
  onSelect: (jobId: string) => void;
  onDismissed: (jobId: string) => void;
}) {
  return (
    <details
      className="shrink-0 border-t border-border-subtle"
      open={jobs.length > 0 && selectedId === null ? true : undefined}
    >
      <summary className="flex h-(--h-toolbar) cursor-default items-center gap-2 bg-surface-window px-3 text-xs select-none">
        <h2 className="font-semibold">Session comparisons · {jobs.length}</h2>
        <span className="text-2xs text-text-muted">
          Native jobs in memory; results expire and nothing is persisted.
        </span>
        <Button
          className="ml-auto"
          variant="ghost"
          size="icon-sm"
          aria-label="Refresh comparisons"
          onClick={(event) => {
            event.preventDefault();
            void observer.refresh();
          }}
        >
          <IconRefresh />
        </Button>
      </summary>
      {jobs.length ? (
        <ul className="max-h-48 overflow-auto">
          {jobs.map((job) => (
            <JobRow
              key={job.jobId}
              job={job}
              selected={job.jobId === selectedId}
              connections={connections}
              observer={observer}
              onSelect={() => onSelect(job.jobId)}
              onDismissed={() => onDismissed(job.jobId)}
            />
          ))}
        </ul>
      ) : (
        <p className="p-(--pad-panel) text-xs text-text-muted">
          No comparisons in this session.
        </p>
      )}
    </details>
  );
}

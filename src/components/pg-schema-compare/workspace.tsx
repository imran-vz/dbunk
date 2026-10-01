import { IconArrowsDiff } from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { Button } from "@/components/ui/button";
import {
  EmptyState,
  ErrorState,
  LoadingBar,
  LoadingState,
} from "@/components/ui/state-panel";
import { schemaCompareClient } from "@/lib/pg-schema-compare/client";
import { isSchemaCompareActive } from "@/lib/pg-schema-compare/failure";
import {
  pgSchemaCompareObserver,
  type SchemaCompareObserver,
} from "@/lib/pg-schema-compare/observer";
import {
  formatSchemaCompareFailure,
  formatSchemaCompareJobFailure,
  PHASE_LABEL,
} from "@/lib/pg-schema-compare/presentation";
import type { SchemaCompareStatus } from "@/lib/pg-schema-compare/protocol";
import {
  createSchemaCompareReader,
  type SchemaCompareSide,
} from "@/lib/pg-schema-compare/reader";
import { type Connection, useAppStore } from "@/lib/store";
import { isTauri } from "@/lib/tauri";

import { CoverageSummary } from "./coverage";
import { EndpointForm, EndpointIdentity } from "./endpoint-form";
import { FieldMatrix } from "./field-matrix";
import { SchemaCompareJobList } from "./job-list";
import { ObjectList } from "./object-list";
import { type EndpointDraft, useSchemaCompareForm } from "./use-compare-form";

const UNAVAILABLE =
  "It may have expired, been dismissed or been invalidated by a connection change. Run a new comparison to capture the current definitions.";

const sameEndpoints = (
  job: SchemaCompareStatus,
  draft: Record<SchemaCompareSide, EndpointDraft>,
) =>
  job.source.connectionId === draft.source.connectionId &&
  job.source.schema === draft.source.schema &&
  job.target.connectionId === draft.target.connectionId &&
  job.target.schema === draft.target.schema;

/** Phase text and returned object counts; never a percentage or a spinner. */
function JobProgress({
  job,
  onCancel,
}: {
  job: SchemaCompareStatus;
  onCancel: () => void;
}) {
  const cancelling = job.phase === "cancelling";
  return (
    <output className="flex flex-wrap items-center gap-x-3 gap-y-1 p-3 text-xs">
      <span className="text-text-muted">{PHASE_LABEL[job.phase]}</span>
      <span className="font-mono text-text-secondary tabular-nums">
        Source: {job.sourceObjects} objects read · Target: {job.targetObjects}{" "}
        objects read
      </span>
      <Button
        size="sm"
        variant="outline"
        disabled={cancelling}
        onClick={onCancel}
      >
        {cancelling ? "Cancelling…" : "Cancel"}
      </Button>
    </output>
  );
}

/**
 * Read-only PostgreSQL 16 schema comparison (Plan 022, Object inspector).
 * The native backend owns jobs and results; this view owns one draft, one
 * selected job and one bounded reader whose payloads it drops on unmount.
 */
export function SchemaCompareWorkspace({
  connection,
  client = schemaCompareClient,
  observer = pgSchemaCompareObserver,
}: {
  connection: Connection;
  client?: typeof schemaCompareClient;
  observer?: SchemaCompareObserver;
}) {
  const connections = useAppStore((state) => state.connections);
  const schemaExplorer = useAppStore((state) => state.schemaExplorer);
  const postgresConnections = connections.filter(
    (candidate) => candidate.engine === "PostgreSQL",
  );
  const form = useSchemaCompareForm(connection, observer);
  const [reader] = useState(() => createSchemaCompareReader(client));
  const view = useStore(reader.store);
  useEffect(() => observer.consume(), [observer]);
  useEffect(() => () => reader.close(), [reader]);
  // Connection edits and deletions invalidate native jobs; reconcile at once.
  useEffect(() => {
    void observer.refresh();
  }, [observer, connections]);

  const { jobs, observedAt } = form.observation;
  const selectedJob = jobs.find((job) => job.jobId === form.selectedJobId);
  const hasConnection = (id: string) => connections.some((c) => c.id === id);
  // A job whose endpoint connection is gone is invalid before native says so.
  const selectedInvalid =
    selectedJob !== undefined &&
    !(
      hasConnection(selectedJob.source.connectionId) &&
      hasConnection(selectedJob.target.connectionId)
    );
  const selectedMissing =
    (form.selectedJobId !== null && observedAt !== null && !selectedJob) ||
    selectedInvalid;
  const openJob = selectedInvalid ? undefined : selectedJob;

  // A completed job opens its immutable result; anything else drops the view.
  useEffect(() => {
    const current = reader.store.getState().request;
    if (openJob?.phase === "completed") {
      if (
        current?.identity.jobId === openJob.jobId &&
        current.identity.resultId === openJob.resultId
      )
        return;
      reader.open({
        identity: { jobId: openJob.jobId, resultId: openJob.resultId },
        source: openJob.source,
        target: openJob.target,
      });
    } else if (current) {
      reader.close();
    }
  }, [reader, openJob]);

  const native = isTauri();
  const uncertain = form.pendingRequest !== null && !form.submitting;
  const controlsDisabled = form.submitting || uncertain || !native;
  const schemasFor = (connectionId: string) =>
    (schemaExplorer[connectionId] ?? []).map((schema) => schema.name);

  const renderResult = () => {
    if (!native) {
      return (
        <EmptyState
          title="Native runtime required"
          description="Schema comparison runs in the desktop app."
        />
      );
    }
    if (form.submitting) {
      return (
        <EmptyState
          title="Starting comparison"
          description="Waiting for native admission."
        />
      );
    }
    if (uncertain) {
      return (
        <EmptyState
          title="Admission not confirmed"
          description="The start response was lost. The comparison may still have been admitted; observation confirms it by request before another start is allowed."
          action={
            <Button
              size="sm"
              variant="outline"
              onClick={() => void observer.refresh()}
            >
              Retry observation
            </Button>
          }
        />
      );
    }
    if (!openJob) {
      return selectedMissing ? (
        <EmptyState
          title="This comparison is unavailable"
          description={UNAVAILABLE}
        />
      ) : (
        <EmptyState
          title="Compare two schemas"
          description="Choose a source and a target, then Compare. Read-only: ordinary table definitions on PostgreSQL 16, with equality reported only within that scope."
        />
      );
    }
    switch (openJob.phase) {
      case "resolving":
      case "readingSource":
      case "readingTarget":
      case "readingBoth":
      case "comparing":
      case "cancelling":
        return (
          <JobProgress
            job={openJob}
            onCancel={() => void observer.cancel(openJob.jobId)}
          />
        );
      case "cancelled":
        return (
          <EmptyState
            title="Comparison cancelled"
            description="No result was produced. Dismiss the job below or compare again."
          />
        );
      case "failed":
        return (
          <div className="flex flex-col items-start gap-2 p-3">
            <ErrorState
              className="self-stretch"
              message={formatSchemaCompareJobFailure(openJob.failure)}
            />
            {openJob.failure.kind === "captureChanged" ? (
              <Button
                size="sm"
                variant="outline"
                disabled={controlsDisabled}
                onClick={() => void form.rerun(openJob)}
              >
                Run again
              </Button>
            ) : null}
          </div>
        );
      case "completed":
        break;
    }
    if (view.unavailable) {
      return (
        <EmptyState
          title="This result is unavailable"
          description={UNAVAILABLE}
          action={
            <Button size="sm" variant="outline" onClick={() => reader.retry()}>
              Retry
            </Button>
          }
        />
      );
    }
    if (!view.metadata) {
      return view.error ? (
        <ErrorState
          message={formatSchemaCompareFailure(view.error)}
          onRetry={() => reader.retry()}
          className="m-3"
        />
      ) : (
        <LoadingState label="Reading result metadata…" />
      );
    }
    return (
      <>
        <CoverageSummary detail={view.metadata} />
        {view.error ? (
          <ErrorState
            message={formatSchemaCompareFailure(view.error)}
            onRetry={() => reader.retry()}
          />
        ) : null}
        {view.metadata.objectCount === 0 ? (
          <EmptyState
            title="No ordinary tables to compare"
            description="Both schemas were read; their objects are outside this comparison scope. This does not mean the schemas are equal. Review exclusions under Coverage & capture."
          />
        ) : (
          <div className="relative flex min-h-0 flex-1 flex-col @3xl:flex-row">
            {view.loading === "objects" || view.loading === "metadata" ? (
              <LoadingBar />
            ) : null}
            <div className="flex max-h-[40%] min-h-0 shrink-0 flex-col border-b border-border-subtle @3xl:max-h-none @3xl:min-w-45 @3xl:flex-[0_1_260px] @3xl:border-r @3xl:border-b-0">
              {view.objects ? (
                <ObjectList
                  page={view.objects}
                  total={view.metadata.objectCount}
                  selected={view.selectedObject}
                  onSelect={(object) => reader.selectObject(object)}
                  onPage={(direction) => reader.objectPage(direction)}
                />
              ) : (
                <p className="p-(--pad-panel) text-xs text-text-muted">
                  Reading objects…
                </p>
              )}
            </div>
            {view.selectedObject ? (
              <FieldMatrix
                schema={{
                  source: openJob.source.schema,
                  target: openJob.target.schema,
                }}
                object={view.selectedObject}
                page={view.fields}
                eligibility={view.eligibility}
                selected={view.selectedField}
                values={view.values}
                loading={
                  view.loading === "fields" ||
                  view.loading === "eligibility" ||
                  view.loading === "values"
                    ? view.loading
                    : null
                }
                onSelect={(field) => reader.selectField(field)}
                onPage={(direction) => reader.fieldPage(direction)}
                onChunk={(side, direction) =>
                  reader.valueChunk(side, direction)
                }
              />
            ) : (
              <p className="flex-1 p-(--pad-panel) text-xs text-text-muted">
                Select an object to inspect its fields.
              </p>
            )}
          </div>
        )}
      </>
    );
  };

  return (
    <div
      className="pg-schema-compare-workspace flex min-h-0 flex-1 flex-col bg-surface-app text-foreground"
      data-testid="pg-schema-compare-workspace"
    >
      <header className="flex min-h-(--h-toolbar) shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-1">
        <IconArrowsDiff className="size-4 text-text-muted" />
        <h1 className="text-sm font-semibold">Schema compare</h1>
        <span className="text-2xs text-text-muted">
          Read-only · PostgreSQL 16 ordinary tables
        </span>
      </header>
      <EndpointForm
        source={form.source}
        target={form.target}
        connections={postgresConnections}
        schemasFor={schemasFor}
        disabled={controlsDisabled}
        onChange={form.setEndpoint}
        action={
          <Button
            size="sm"
            disabled={controlsDisabled}
            onClick={() => void form.submit()}
          >
            {form.submitting ? "Starting…" : "Compare"}
          </Button>
        }
      />
      {form.observation.error ? (
        <ErrorState
          message={formatSchemaCompareFailure(form.observation.error)}
          onRetry={() => void observer.refresh()}
        />
      ) : null}
      {form.error ? (
        <p
          role="alert"
          className="border-b border-border-subtle px-3 py-1.5 text-xs text-danger"
        >
          {form.error}
        </p>
      ) : null}
      {openJob ? (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-border-subtle bg-surface-sidebar px-3 py-1 text-xs">
          <span className="text-2xs font-semibold tracking-wide text-text-muted uppercase">
            {isSchemaCompareActive(openJob) ? "Comparing" : "Result"}
          </span>
          <EndpointIdentity
            endpoint={openJob.source}
            connections={connections}
          />
          <span aria-hidden="true" className="text-text-muted">
            →
          </span>
          <EndpointIdentity
            endpoint={openJob.target}
            connections={connections}
          />
          {sameEndpoints(openJob, {
            source: form.source,
            target: form.target,
          }) ? null : (
            <span className="text-text-muted">
              Draft endpoints differ from this comparison.
            </span>
          )}
        </div>
      ) : null}
      <div className="@container flex min-h-0 flex-1 flex-col">
        {renderResult()}
      </div>
      <SchemaCompareJobList
        jobs={jobs}
        selectedId={form.selectedJobId}
        connections={connections}
        observer={observer}
        onSelect={form.selectJob}
        onDismissed={(jobId) => {
          if (form.selectedJobId === jobId) form.selectJob(null);
        }}
      />
    </div>
  );
}

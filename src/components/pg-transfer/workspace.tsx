import {
  IconDownload,
  IconFolder,
  IconTransfer,
  IconUpload,
} from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import {
  formatPgTransferError,
  isPgTransferJobActive,
} from "@/lib/pg-transfer/client";
import { pgTransferObserver } from "@/lib/pg-transfer/observer";
import { ENVIRONMENT_META } from "@/lib/safety-policy";
import { isConnectedStatus, type Connection } from "@/lib/store";
import { isTauri } from "@/lib/tauri";

import { CsvOptionsFields } from "./csv-options";
import { PgCsvImportReview, PgCsvSample } from "./import-review";
import { PgTransferJobDetails, PgTransferJobList } from "./job-list";
import { type PgTransferIntent, useTransferForm } from "./use-transfer-form";

const fileName = (path: string | null) =>
  path?.split(/[/\\]/).filter(Boolean).at(-1) ?? "No file selected";

export function PgTransferWorkspace({
  connection,
  schema,
  table,
  intent,
}: {
  connection: Connection;
  schema: string;
  table: string;
  intent?: PgTransferIntent;
}) {
  const form = useTransferForm(connection, { schema, table }, intent);
  const observation = useStore(pgTransferObserver.store);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  useEffect(() => pgTransferObserver.consume(), []);
  const jobs = observation.jobs.filter(
    (job) => job.connectionId === connection.id,
  );
  const selected = jobs.find((job) => job.jobId === (selectedId ?? form.jobId));
  const active = jobs.some(isPgTransferJobActive);
  const disabled =
    form.busy !== null ||
    Boolean(form.closing) ||
    active ||
    Boolean(observation.error) ||
    !isConnectedStatus(connection.status) ||
    !isTauri();
  const reviewed = form.inspection !== null;
  const canStart =
    reviewed && form.path !== null && form.validation === null && !disabled;

  return (
    <div
      className="pg-transfer-workspace flex min-h-0 flex-1 flex-col bg-surface-app text-foreground"
      data-testid="pg-transfer-workspace"
    >
      <header className="flex min-h-(--h-toolbar) shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-1">
        <IconTransfer className="size-4 text-text-muted" />
        <h1 className="text-sm font-semibold">CSV transfer</h1>
        <span className="font-mono text-2xs text-text-muted">
          {schema}.{table}
        </span>
        <div className="ml-auto">
          <Segmented
            options={[
              { id: "import", label: "Import", icon: <IconUpload /> },
              { id: "export", label: "Export", icon: <IconDownload /> },
            ]}
            value={form.direction}
            onChange={(direction) => {
              setSelectedId(null);
              form.changeDirection(direction);
            }}
          />
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-auto">
        {observation.error ? (
          <div
            role="alert"
            className="flex items-center gap-2 border-b border-border-subtle p-3 text-xs text-danger"
          >
            {formatPgTransferError(observation.error)}
            <Button
              size="sm"
              variant="outline"
              onClick={() => void pgTransferObserver.refresh()}
            >
              Retry observation
            </Button>
          </div>
        ) : null}

        <div className="grid grid-cols-1 lg:grid-cols-2">
          <form
            className="min-w-0 p-4 lg:p-6"
            onSubmit={(event) => {
              event.preventDefault();
              void form.submit();
            }}
          >
            <h2 className="mb-4 text-md font-semibold">
              {form.direction === "import" ? "Import CSV" : "Export CSV"}
            </h2>
            <div className="mb-3 grid grid-cols-[6rem_minmax(0,1fr)] gap-3 text-xs">
              <span className="text-text-secondary">
                {form.direction === "import" ? "Target" : "Source"}
              </span>
              <span className="break-all font-mono">
                {connection.database}.{schema}.{table}
              </span>
            </div>
            <div className="mb-4 grid grid-cols-[6rem_minmax(0,1fr)] gap-3 text-xs">
              <span className="text-text-secondary">Environment</span>
              <span>
                {
                  ENVIRONMENT_META[connection.environment ?? "development"]
                    .label
                }
              </span>
            </div>
            <div className="mb-4 grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
              <span className="pt-1 text-text-secondary">
                {form.direction === "import" ? "Source file" : "Destination"}
              </span>
              <div className="min-w-0">
                <div className="flex gap-1.5">
                  <Input
                    aria-label={
                      form.direction === "import"
                        ? "CSV source file"
                        : "CSV destination"
                    }
                    value={form.inspection?.fileName ?? fileName(form.path)}
                    readOnly
                    className="min-w-0 font-mono text-xs"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    disabled={disabled}
                    onClick={() => void form.pick()}
                  >
                    <IconFolder />
                    {form.busy === "picking" ? "Choosing…" : "Browse…"}
                  </Button>
                </div>
                <p className="mt-1 text-2xs text-text-muted">
                  {form.direction === "import"
                    ? form.inspection?.totalBytes === null ||
                      form.inspection?.totalBytes === undefined
                      ? "Keep the source unchanged during review and import."
                      : `${form.inspection.totalBytes.toLocaleString()} bytes · Keep the source unchanged during import.`
                    : "Choose a fresh file. Existing destinations are never replaced."}
                </p>
              </div>
            </div>

            <CsvOptionsFields
              direction={form.direction}
              options={form.options}
              disabled={disabled}
              onChange={form.changeOptions}
            />

            {form.direction === "import" && form.inspection ? (
              <PgCsvImportReview
                inspection={form.inspection}
                mapping={form.mapping}
                validation={form.validation}
                disabled={disabled}
                onChange={form.setMapping}
              />
            ) : null}

            <p className="mt-4 text-2xs text-text-muted">
              {form.direction === "import"
                ? "Append only · One transaction · Stop on first error · ISO dates · UTC"
                : "Whole committed table · Includes partitions · Row order unspecified"}
            </p>
            {form.error ? (
              <p role="alert" className="mt-3 text-xs text-danger">
                {form.error}
              </p>
            ) : null}
            {!isConnectedStatus(connection.status) ? (
              <p className="mt-3 text-xs text-text-secondary">
                Connect to inspect or start. Existing transfers remain visible.
              </p>
            ) : null}
            {!isTauri() ? (
              <p className="mt-3 text-xs text-text-secondary">
                Native CSV transfer requires the desktop app.
              </p>
            ) : null}
            {active ? (
              <p className="mt-3 text-xs text-text-secondary">
                This connection already has an active transfer.
              </p>
            ) : null}
            <div className="mt-4 flex items-center justify-end gap-2 border-t border-border-subtle pt-3">
              {!reviewed &&
              (form.direction === "export" || form.path !== null) ? (
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  disabled={disabled}
                  onClick={() => void form.inspect()}
                >
                  {form.busy === "inspecting"
                    ? "Inspecting…"
                    : form.direction === "import"
                      ? "Inspect again"
                      : "Review export"}
                </Button>
              ) : null}
              <Button type="submit" size="sm" disabled={!canStart}>
                {form.direction === "import" ? (
                  <IconUpload />
                ) : (
                  <IconDownload />
                )}
                {form.busy === "starting"
                  ? "Starting…"
                  : form.direction === "import"
                    ? "Import CSV…"
                    : "Export CSV"}
              </Button>
            </div>
          </form>

          <div className="min-w-0 border-t border-border-subtle lg:border-t-0 lg:border-l">
            {selected ? (
              <PgTransferJobDetails key={selected.jobId} job={selected} />
            ) : form.inspection?.direction === "import" ? (
              <aside className="p-4 lg:p-6">
                <h2 className="text-sm font-semibold">Review import</h2>
                <PgCsvSample inspection={form.inspection} />
                <TransferSafety direction="import" />
              </aside>
            ) : (
              <aside className="p-4 text-xs text-text-secondary lg:p-6">
                {(selectedId ?? form.jobId) ? (
                  <output className="mb-3 block">
                    This transfer expired or is unavailable. Refresh observation
                    to check again.
                  </output>
                ) : null}
                <h2 className="mb-3 font-semibold text-foreground">
                  {form.direction === "import"
                    ? "Import details"
                    : reviewed
                      ? "Review export"
                      : "Whole-table export"}
                </h2>
                {form.direction === "export" && reviewed ? (
                  <dl className="mb-4">
                    <div className="flex justify-between gap-4 border-b border-border-subtle py-2">
                      <dt className="text-text-muted">Columns</dt>
                      <dd>{form.inspection?.targetColumns.length ?? 0}</dd>
                    </div>
                    <div className="flex justify-between gap-4 border-b border-border-subtle py-2">
                      <dt className="text-text-muted">Rows</dt>
                      <dd>Whole table</dd>
                    </div>
                  </dl>
                ) : null}
                <TransferSafety direction={form.direction} />
              </aside>
            )}
          </div>
        </div>

        <PgTransferJobList
          jobs={jobs}
          selectedId={selected?.jobId ?? null}
          onSelect={setSelectedId}
        />
      </div>
    </div>
  );
}

function TransferSafety({ direction }: { direction: "import" | "export" }) {
  return (
    <div className="mt-4 space-y-3 text-xs text-text-secondary">
      {direction === "import" ? (
        <>
          <p>
            Existing rows stay unchanged. Constraints, defaults and triggers run
            normally. Unquoted NULL tokens become SQL NULL; quoted tokens remain
            text.
          </p>
          <p>
            Rollback cannot reverse sequence increments or external trigger
            effects.
          </p>
        </>
      ) : (
        <>
          <p>
            Grid filters, loaded rows, selection and staged edits do not affect
            this export.
          </p>
          <p>
            Data streams to a private partial file. The destination appears only
            after a successful sync and publication. A hard crash can leave the
            partial file beside it.
          </p>
        </>
      )}
      <p className="border-l-2 border-border-strong pl-3">
        Closing this table keeps the transfer running. Saving or disconnecting
        the connection stops active work.
      </p>
    </div>
  );
}

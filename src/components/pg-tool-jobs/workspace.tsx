import {
  IconArchive,
  IconDownload,
  IconFolder,
  IconUpload,
} from "@tabler/icons-react";
import { useEffect, useState } from "react";
import { useStore } from "zustand";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Segmented } from "@/components/ui/segmented";
import {
  formatPgToolError,
  isPgToolJobActive,
  type PgBackupScope,
  type PgToolJob,
} from "@/lib/pg-tool-jobs/client";
import { pgToolObserver } from "@/lib/pg-tool-jobs/observer";
import { ENVIRONMENT_META } from "@/lib/safety-policy";
import { type Connection, isConnectedStatus, useAppStore } from "@/lib/store";
import { isTauri } from "@/lib/tauri";

import { PgToolJobDetails, PgToolJobList } from "./job-list";
import { useToolForm } from "./use-tool-form";

const selectClass =
  "h-(--control-h) w-full rounded-sm border border-border-strong bg-surface-input px-2 text-xs outline-none focus-visible:ring-1 focus-visible:ring-accent";
export function PgToolWorkspace({
  connection,
  table,
  initialOperation = "backup",
}: {
  connection: Connection;
  table?: { schema: string; name: string };
  initialOperation?: PgToolJob["kind"];
}) {
  const initialScope: PgBackupScope = table
    ? { kind: "table", schema: table.schema, table: table.name }
    : { kind: "database" };
  const form = useToolForm(connection, initialScope, initialOperation);
  const observation = useStore(pgToolObserver.store);
  const catalog = useAppStore((s) => s.pgObjectCatalog[connection.id]?.catalog);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  useEffect(() => pgToolObserver.consume(), []);
  const jobs = observation.jobs.filter((j) => j.connectionId === connection.id);
  const selected = jobs.find((j) => j.jobId === (selectedId ?? form.jobId));
  const active = jobs.some(isPgToolJobActive);
  const backup = form.operation === "backup";
  const cleanEnabled = backup
    ? form.format === "plain"
    : form.format === "custom";
  const disabled =
    form.busy ||
    Boolean(form.closing) ||
    active ||
    Boolean(observation.error) ||
    !isConnectedStatus(connection.status) ||
    !isTauri();
  const schema = form.scope.kind === "database" ? "" : form.scope.schema;
  const scopeValid =
    form.scope.kind === "database" ||
    (Boolean(schema) &&
      (form.scope.kind !== "table" || Boolean(form.scope.table)));
  const fileName = form.path?.split(/[/\\]/).pop() ?? "No file selected";
  return (
    <div
      className="pg-tool-workspace flex min-h-0 flex-1 flex-col bg-surface-app text-foreground"
      data-testid="pg-tool-workspace"
    >
      <header className="flex min-h-(--h-toolbar) shrink-0 flex-wrap items-center gap-2 border-b border-border-subtle bg-surface-window px-3 py-1">
        <IconArchive className="size-4 text-text-muted" />
        <h1 className="text-sm font-semibold">Backup / Restore</h1>
        <span className="text-2xs text-text-muted">{connection.database}</span>
        <div className="ml-auto">
          <Segmented
            options={[
              { id: "backup", label: "Backup" },
              { id: "restore", label: "Restore" },
            ]}
            value={form.operation}
            onChange={form.changeOperation}
          />
        </div>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {observation.error ? (
          <div
            role="alert"
            className="flex items-center gap-2 border-b border-border-subtle p-3 text-xs text-danger"
          >
            {formatPgToolError(observation.error)}
            <Button
              size="sm"
              variant="outline"
              onClick={() => void pgToolObserver.refresh()}
            >
              Retry observation
            </Button>
          </div>
        ) : null}
        <div className="grid grid-cols-1 lg:grid-cols-2">
          <form
            className="min-w-0 p-4 lg:p-6"
            onSubmit={(e) => {
              e.preventDefault();
              void form.submit();
            }}
          >
            <h2 className="mb-4 text-md font-semibold">
              {backup ? "Back up" : "Restore"} database
            </h2>
            <div className="mb-4 grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
              <span className="text-text-secondary">
                {backup ? "Source" : "Target"}
              </span>
              <span className="break-all font-mono">
                {connection.database} · {connection.name}
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
            <fieldset disabled={disabled} className="min-w-0 space-y-4">
              <label className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-3 text-xs">
                <span className="text-text-secondary">Format</span>
                <select
                  aria-label="Archive format"
                  className={selectClass}
                  value={form.format}
                  onChange={(e) =>
                    form.changeFormat(
                      e.target.value === "plain" ? "plain" : "custom",
                    )
                  }
                >
                  <option value="custom">Custom archive</option>
                  <option value="plain">Plain SQL</option>
                </select>
              </label>
              {backup ? (
                <>
                  <label className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-3 text-xs">
                    <span className="text-text-secondary">Scope</span>
                    <select
                      aria-label="Backup scope"
                      className={selectClass}
                      value={form.scope.kind}
                      onChange={(e) =>
                        form.setScope(
                          e.target.value === "table"
                            ? { kind: "table", schema: "", table: "" }
                            : e.target.value === "schema"
                              ? { kind: "schema", schema: "" }
                              : { kind: "database" },
                        )
                      }
                    >
                      <option value="database">Entire database</option>
                      <option value="schema">Schema</option>
                      <option value="table">Table</option>
                    </select>
                  </label>
                  {form.scope.kind !== "database" ? (
                    <label className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-3 text-xs">
                      <span className="text-text-secondary">Schema</span>
                      <select
                        aria-label="Backup schema"
                        className={selectClass}
                        value={schema}
                        onChange={(e) =>
                          form.setScope(
                            form.scope.kind === "table"
                              ? {
                                  kind: "table",
                                  schema: e.target.value,
                                  table: "",
                                }
                              : { kind: "schema", schema: e.target.value },
                          )
                        }
                      >
                        <option value="">Choose schema</option>
                        {catalog?.schemas.map((s) => (
                          <option key={s.name}>{s.name}</option>
                        ))}
                      </select>
                    </label>
                  ) : null}
                  {form.scope.kind === "table" ? (
                    <label className="grid grid-cols-[6rem_minmax(0,1fr)] items-center gap-3 text-xs">
                      <span className="text-text-secondary">Table</span>
                      <select
                        aria-label="Backup table"
                        className={selectClass}
                        value={form.scope.table}
                        onChange={(e) =>
                          form.setScope({
                            kind: "table",
                            schema,
                            table: e.target.value,
                          })
                        }
                      >
                        <option value="">Choose table</option>
                        {catalog?.schemas
                          .find((s) => s.name === schema)
                          ?.tables.map((t) => (
                            <option key={t.name}>{t.name}</option>
                          ))}
                      </select>
                    </label>
                  ) : null}
                </>
              ) : null}
              <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
                <span className="pt-1 text-text-secondary">
                  {backup ? "Destination" : "Source file"}
                </span>
                <div className="min-w-0">
                  <div className="flex gap-1.5">
                    <Input
                      aria-label={
                        backup ? "Backup destination" : "Restore source"
                      }
                      value={fileName}
                      readOnly
                      className="min-w-0 font-mono text-xs"
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={() => void form.pick()}
                    >
                      <IconFolder />
                      Browse…
                    </Button>
                  </div>
                  <p className="mt-1 text-2xs text-text-muted">
                    {backup
                      ? "Choose a new file. Existing files are never replaced."
                      : "Select the format explicitly; filenames do not determine format."}
                  </p>
                </div>
              </div>
              <div className="grid grid-cols-[6rem_minmax(0,1fr)] items-start gap-3 text-xs">
                <span className="pt-1 text-text-secondary">Options</span>
                <div>
                  <label className="flex items-start gap-2">
                    <input
                      type="checkbox"
                      disabled={!cleanEnabled}
                      checked={cleanEnabled && form.clean}
                      onChange={(e) => form.setClean(e.target.checked)}
                    />
                    {backup
                      ? "Include drop statements"
                      : "Drop existing objects before restore"}
                  </label>
                  <p className="mt-1 text-2xs text-text-muted">
                    {backup
                      ? cleanEnabled
                        ? "Embeds cleanup commands for a future restore."
                        : "Available for plain SQL backups."
                      : cleanEnabled
                        ? "Runs before restore in the same transaction."
                        : "Cleanup depends on the SQL file."}
                  </p>
                </div>
              </div>
            </fieldset>
            {!backup ? (
              <p className="mt-4 border-l-2 border-warning bg-accent-subdued p-2 text-xs text-warning">
                Only restore trusted files. SQL runs with this connection’s
                privileges.
                {table
                  ? " This targets the database, not only this table."
                  : ""}
              </p>
            ) : null}
            {form.error ? (
              <p role="alert" className="mt-3 text-xs text-danger">
                {form.error}
              </p>
            ) : null}
            {!backup && connection.readOnly ? (
              <p className="mt-3 text-xs text-warning">
                Restore is unavailable on a read-only connection.
              </p>
            ) : null}
            {!isConnectedStatus(connection.status) ? (
              <p className="mt-3 text-xs text-text-secondary">
                Connect to start a job. Existing jobs remain visible.
              </p>
            ) : null}
            {!isTauri() ? (
              <p className="mt-3 text-xs text-text-secondary">
                Native backup and restore require the desktop app.
              </p>
            ) : null}
            <div className="mt-4 flex justify-end border-t border-border-subtle pt-3">
              <Button
                type="submit"
                size="sm"
                disabled={
                  disabled ||
                  !form.path ||
                  (backup && !scopeValid) ||
                  (!backup && connection.readOnly)
                }
              >
                {backup ? <IconDownload /> : <IconUpload />}
                {form.busy
                  ? "Starting…"
                  : backup
                    ? "Back up"
                    : "Restore database…"}
              </Button>
            </div>
          </form>
          <div className="min-w-0 border-t border-border-subtle lg:border-t-0 lg:border-l">
            {selected ? (
              <PgToolJobDetails key={selected.jobId} job={selected} />
            ) : (
              <aside className="p-4 text-xs text-text-secondary lg:p-6">
                {(selectedId ?? form.jobId) ? (
                  <output className="mb-3 block">
                    This job is expired or unavailable. Refresh observation to
                    check again.
                  </output>
                ) : null}
                <h2 className="mb-3 font-semibold text-foreground">
                  {backup
                    ? "Native PostgreSQL backup"
                    : "Restore into this database"}
                </h2>
                <p>
                  {backup
                    ? "The archive is saved only after the backup finishes successfully."
                    : "Existing objects or missing owner roles can make restore fail. This does not create a database or remap owners."}
                </p>
                <p className="mt-3">
                  Saving connection settings cancels active jobs.
                </p>
                <details className="mt-4">
                  <summary className="cursor-pointer text-foreground">
                    Limits and client tools
                  </summary>
                  <p className="mt-2">
                    History is in memory, up to 1 hour and 32 finished jobs. A
                    hard crash can leave a hidden partial archive. Never promote
                    it to a completed backup.
                  </p>
                  <p className="mt-2">
                    Requires patched PostgreSQL clients: 14.24+, 15.19+, 16.15+,
                    17.11+, 18.6+, or a newer major. pg_dump must also support
                    the server version. Client tools are discovered through PATH
                    and standard PostgreSQL locations.
                  </p>
                </details>
              </aside>
            )}
          </div>
        </div>
        <PgToolJobList
          jobs={jobs}
          selectedId={selected?.jobId ?? null}
          onSelect={setSelectedId}
        />
      </div>
    </div>
  );
}

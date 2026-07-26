import {
  IconDatabaseExport,
  IconDatabaseImport,
  IconEdit,
} from "@tabler/icons-react";
import { type ReactNode, useRef, useState } from "react";
import { toast } from "sonner";

import { EditConnectionDialog } from "@/components/edit-connection-dialog";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { base64ToBytes, fileToBase64 } from "@/lib/backup";
import { downloadBlob, downloadFile } from "@/lib/download";
import type { Connection, PgDriverOptions } from "@/lib/store";
import { errorToMessage, isTauri, tauriInvoke } from "@/lib/tauri";

import { KeyValue } from "./key-value";

/**
 * Settings sub-tab — read-only mirror of the connection's configured
 * fields plus an Edit button that opens the connection dialog.
 *
 * Editing deliberately stays in that one dialog rather than becoming a
 * second inline editor here: `<ConnectionForm>` is the single
 * construction site for the `StoredConnection` wire shape (ADR-0012),
 * and it already routes saves through `updateConnection`. The mirror's
 * job is to show what's set — including the ADR-0013 driver knobs,
 * which live behind the dialog's Advanced expander and would otherwise
 * be invisible from the workspace.
 */
export function SettingsTab({ connection }: { connection: Connection }) {
  const [isEditing, setIsEditing] = useState(false);
  const [backupError, setBackupError] = useState<string | null>(null);
  const [backupBusy, setBackupBusy] = useState<string | null>(null);
  const [restoreClean, setRestoreClean] = useState(false);
  const restoreInputRef = useRef<HTMLInputElement | null>(null);

  const rows = buildSettingsRows(connection);

  return (
    <>
      <Card>
        <CardHeader>
          <CardTitle>Connection settings</CardTitle>
          <CardAction>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setIsEditing(true)}
            >
              <IconEdit className="size-3.5" />
              Edit connection
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-x-6 gap-y-3 text-xs sm:grid-cols-3">
          {rows.map(([label, value]) => (
            <KeyValue key={label} label={label} value={value} />
          ))}
        </CardContent>
      </Card>

      {connection.engine === "PostgreSQL" ? (
        <Card>
          <CardHeader>
            <CardTitle>Backup and restore</CardTitle>
            <CardAction>
              <label className="flex items-center gap-1.5 text-[0.6875rem] text-text-muted">
                <input
                  type="checkbox"
                  checked={restoreClean}
                  onChange={(event) => setRestoreClean(event.target.checked)}
                />
                Clean restore
              </label>
            </CardAction>
          </CardHeader>
          <CardContent className="flex flex-wrap items-center gap-2 text-xs">
            <Button
              size="sm"
              variant="outline"
              disabled={backupBusy !== null}
              onClick={() =>
                void exportDatabaseDdl(
                  connection.id,
                  setBackupBusy,
                  setBackupError,
                )
              }
            >
              <IconDatabaseExport className="size-3.5" />
              Export DDL
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={backupBusy !== null}
              onClick={() =>
                void runDump(connection, "plain", setBackupBusy, setBackupError)
              }
            >
              <IconDatabaseExport className="size-3.5" />
              pg_dump SQL
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={backupBusy !== null}
              onClick={() =>
                void runDump(
                  connection,
                  "custom",
                  setBackupBusy,
                  setBackupError,
                )
              }
            >
              <IconDatabaseExport className="size-3.5" />
              pg_dump custom
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={backupBusy !== null}
              onClick={() => restoreInputRef.current?.click()}
            >
              <IconDatabaseImport className="size-3.5" />
              Restore dump
            </Button>
            <input
              ref={restoreInputRef}
              type="file"
              className="hidden"
              accept=".sql,.dump,.backup,application/sql,application/octet-stream"
              onChange={(event) => {
                const file = event.target.files?.[0];
                event.currentTarget.value = "";
                if (file) {
                  void runRestore(
                    connection.id,
                    file,
                    restoreClean,
                    setBackupBusy,
                    setBackupError,
                  );
                }
              }}
            />
            {backupBusy ? (
              <span className="text-text-muted">{backupBusy}…</span>
            ) : null}
            {backupError ? (
              <span className="text-danger">{backupError}</span>
            ) : null}
          </CardContent>
        </Card>
      ) : null}

      <EditConnectionDialog
        connection={isEditing ? connection : null}
        open={isEditing}
        onOpenChange={(open) => {
          if (!open) setIsEditing(false);
        }}
      />
    </>
  );
}

async function exportDatabaseDdl(
  connectionId: string,
  setBusy: (value: string | null) => void,
  setError: (value: string | null) => void,
) {
  if (!isTauri()) {
    setError("DDL export requires the desktop runtime.");
    return;
  }
  setBusy("Exporting DDL");
  setError(null);
  try {
    const result = await tauriInvoke<{ sql: string }>("export_ddl", {
      payload: { connectionId, scope: "database" },
    });
    downloadFile(
      "database.ddl.sql",
      "application/sql;charset=utf-8",
      result.sql,
    );
  } catch (error) {
    setError(errorToMessage(error));
  } finally {
    setBusy(null);
  }
}

async function runDump(
  connection: Connection,
  format: "plain" | "custom",
  setBusy: (value: string | null) => void,
  setError: (value: string | null) => void,
) {
  if (!isTauri()) {
    setError("pg_dump requires the desktop runtime.");
    return;
  }
  setBusy("Running pg_dump");
  setError(null);
  try {
    const result = await tauriInvoke<{
      dataBase64: string;
      extension: string;
    }>("run_pg_dump", {
      payload: {
        connectionId: connection.id,
        scope: "database",
        format,
      },
    });
    const bytes = new Uint8Array(base64ToBytes(result.dataBase64));
    const filename = `${connection.database || connection.name}.${result.extension}`;
    downloadBlob(
      filename,
      new Blob([bytes], {
        type:
          format === "plain" ? "application/sql" : "application/octet-stream",
      }),
    );
    toast.success(`Saved ${filename}`);
  } catch (error) {
    const message = errorToMessage(error);
    setError(message);
    toast.error(`pg_dump failed: ${message}`);
  } finally {
    setBusy(null);
  }
}

async function runRestore(
  connectionId: string,
  file: File,
  clean: boolean,
  setBusy: (value: string | null) => void,
  setError: (value: string | null) => void,
) {
  if (!isTauri()) {
    setError("pg_restore requires the desktop runtime.");
    return;
  }
  const format = /\.sql$/i.test(file.name) ? "plain" : "custom";
  setBusy("Restoring dump");
  setError(null);
  try {
    await tauriInvoke("run_pg_restore", {
      payload: {
        connectionId,
        dataBase64: await fileToBase64(file),
        format,
        clean,
      },
    });
    toast.success(`Restored ${file.name}`);
  } catch (error) {
    const message = errorToMessage(error);
    setError(message);
    toast.error(`pg_restore failed: ${message}`);
  } finally {
    setBusy(null);
  }
}

const placeholderValue = <span className="text-text-muted">—</span>;

/**
 * Build the engine-aware row list for the settings panel. Common
 * fields render for every relational connection; engine-specific
 * fields (TLS flavour, ClickHouse URL path) trail the common block.
 * Empty/optional values render as a muted dash so the grid stays
 * symmetric across engines.
 */
function buildSettingsRows(connection: Connection): Array<[string, ReactNode]> {
  const rows: Array<[string, ReactNode]> = [
    ["Name", connection.name],
    ["Engine", connection.engine],
    ["Host", connection.host || placeholderValue],
    ["Port", connection.port ? String(connection.port) : placeholderValue],
    ["Database", connection.database || placeholderValue],
    ["User", connection.user || placeholderValue],
    ["Role", connection.role || placeholderValue],
  ];

  if (connection.engine === "PostgreSQL" || connection.engine === "MySQL") {
    rows.push(["SSL", connection.ssl ? "Enabled" : "Disabled"]);
  } else if (connection.engine === "ClickHouse") {
    rows.push(["HTTPS", connection.useHttps ? "Enabled" : "Disabled"]);
    rows.push(["URL path", connection.urlPath || placeholderValue]);
  }
  // SQLite has no TLS / network-config fields beyond the common block.
  // Redis takes the keyvalue workspace branch and never reaches here.

  if (connection.engine === "PostgreSQL") {
    rows.push(...driverOptionRows(connection.driverOptions));
  }

  return rows;
}

/**
 * ADR-0013 knobs. The whole block is omitted when nothing is
 * configured — which is the common case — rather than adding five
 * permanent dashes to every Postgres connection's grid. Once any knob
 * is set the block renders in full, so an unset neighbour reads as
 * "server default" instead of vanishing.
 */
function driverOptionRows(
  options: PgDriverOptions | undefined,
): Array<[string, ReactNode]> {
  if (!options || Object.keys(options).length === 0) {
    return [];
  }
  return [
    ["Statement timeout", millis(options.statementTimeoutMs)],
    ["Idle in transaction", millis(options.idleInTransactionTimeoutMs)],
    ["Connect timeout", millis(options.connectTimeoutMs)],
    ["Search path", options.defaultSearchPath?.join(", ") || placeholderValue],
    ["Default role", options.defaultRole || placeholderValue],
  ];
}

function millis(value: number | undefined): ReactNode {
  if (value === undefined) return placeholderValue;
  // PG reads 0 as "no limit" for the session timeouts; spelling that
  // out beats rendering a bare "0 ms" the user has to look up.
  return value === 0 ? "No limit" : `${value} ms`;
}

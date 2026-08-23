/**
 * Inline Drizzle-Studio-style FK drill-down preview.
 *
 * Rendered inside the data-grid's `rowExpansion` slot — sits directly
 * below the row whose foreign-key cell the user clicked. Shows the
 * referenced row(s) as a compact key/value list so the user can peek
 * without leaving the current grid.
 *
 * Loading + error states render in-place. The header has a back/close
 * button and ESC also closes (handled by the parent component since
 * ESC needs to clear the parent's `inlineDrilldown` state).
 *
 * SQL plumbing mirrors the previous full-page drill-down: identifier
 * + literal quoting is engine-aware (PG `"…"`, ClickHouse backticks).
 */

import { IconArrowRight, IconX } from "@tabler/icons-react";
import { useEffect, useState } from "react";

import type { ForeignKeyTarget } from "@/components/data-grid";
import { Button } from "@/components/ui/button";
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import { type DatabaseEngine, useAppStore } from "@/lib/store";
import { errorToMessage } from "@/lib/tauri";

const ROW_LIMIT = 5;

export interface InlineDrilldownProps {
  connectionId: string;
  engine: DatabaseEngine;
  target: ForeignKeyTarget;
  value: string;
  onClose: () => void;
  /**
   * Hook so the user can keep drilling from inside the preview —
   * clicking an FK in the preview chains to a new drill-down on the
   * same row. The parent decides whether to replace the current
   * inline preview or stack it.
   */
  onFollowForeignKey?: (target: ForeignKeyTarget, value: string) => void;
}

type FetchState =
  | { kind: "loading" }
  | { kind: "loaded"; columns: string[]; rows: string[][] }
  | { kind: "error"; message: string };

export function InlineDrilldown({
  connectionId,
  engine,
  target,
  value,
  onClose,
}: InlineDrilldownProps) {
  const [state, setState] = useState<FetchState>({ kind: "loading" });
  const connection = useAppStore((store) =>
    store.connections.find((candidate) => candidate.id === connectionId),
  );

  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    const sql = buildFilteredSelectSql(engine, target, value);
    invokeWithSafetyConfirmation<{ columns: string[]; rows: string[][] }>({
      command: "run_query",
      connection,
      payload: { connectionId, query: sql },
    })
      .then((result) => {
        if (cancelled) return;
        setState({
          kind: "loaded",
          columns: result.columns,
          rows: result.rows,
        });
      })
      .catch((err) => {
        if (cancelled) return;
        setState({ kind: "error", message: errorToMessage(err) });
      });
    return () => {
      cancelled = true;
    };
  }, [connection, connectionId, engine, target, value]);

  return (
    <div className="border-x border-border-subtle/60 px-3 py-2 text-2xs">
      <div className="flex items-center gap-2 text-text-muted">
        <IconArrowRight className="size-3 text-primary" />
        <span className="font-mono">
          <span className="text-foreground">
            {target.schema}.{target.table}
          </span>
          <span className="text-text-muted"> where </span>
          <span className="text-foreground">{target.column}</span>
          <span className="text-text-muted"> = </span>
          <span className="text-foreground">'{value}'</span>
        </span>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto h-6 px-2 text-2xs"
          onClick={onClose}
          aria-label="Close drill-down"
          title="Close (Esc)"
        >
          <IconX className="size-3.5" />
        </Button>
      </div>
      <div className="mt-2">
        {state.kind === "loading" ? (
          <div className="text-text-muted">Loading…</div>
        ) : state.kind === "error" ? (
          <div className="rounded-md border border-danger/30 bg-danger/10 px-2 py-1 text-danger">
            {state.message}
          </div>
        ) : state.rows.length === 0 ? (
          <div className="text-text-muted">No matching rows.</div>
        ) : (
          <DrilldownRows columns={state.columns} rows={state.rows} />
        )}
      </div>
    </div>
  );
}

function DrilldownRows({
  columns,
  rows,
}: {
  columns: string[];
  rows: string[][];
}) {
  return (
    <div className="space-y-1.5">
      <div className="overflow-auto rounded-md border border-border-subtle bg-surface-app">
        <table className="min-w-full divide-y divide-border-subtle font-mono text-2xs">
          <thead className="bg-surface-panel-elevated/60 text-text-muted">
            <tr>
              {columns.map((col) => (
                <th
                  key={col}
                  scope="col"
                  className="whitespace-nowrap border-r border-border-subtle px-2 py-1 text-left font-normal last:border-r-0"
                >
                  {col}
                </th>
              ))}
            </tr>
          </thead>
          <tbody className="divide-y divide-border-subtle">
            {rows.map((row, rowIdx) => (
              <tr
                // oxlint-disable-next-line react/no-array-index-key -- positional row, no stable id
                key={`row-${rowIdx}`}
                className="hover:bg-surface-row-hover"
              >
                {columns.map((col, colIdx) => {
                  const cell = row[colIdx];
                  return (
                    <td
                      key={col}
                      className="max-w-[18rem] truncate border-r border-border-subtle px-2 py-1 text-foreground last:border-r-0"
                      title={cell ?? "NULL"}
                    >
                      {cell ?? <span className="text-text-muted">NULL</span>}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {rows.length === ROW_LIMIT ? (
        <div className="text-2xs text-text-muted">
          Showing first {ROW_LIMIT} matching rows.
        </div>
      ) : null}
    </div>
  );
}

function buildFilteredSelectSql(
  engine: DatabaseEngine,
  target: ForeignKeyTarget,
  value: string,
): string {
  const ident = engine === "ClickHouse" ? quoteIdentBacktick : quoteIdentDouble;
  const qualified = `${ident(target.schema)}.${ident(target.table)}`;
  const colExpr = ident(target.column);
  const literal = quoteStringLiteral(value);
  return `SELECT * FROM ${qualified} WHERE ${colExpr} = ${literal} LIMIT ${ROW_LIMIT}`;
}

function quoteIdentDouble(name: string): string {
  return `"${name.replaceAll('"', '""')}"`;
}

function quoteIdentBacktick(name: string): string {
  return `\`${name.replaceAll("`", "``")}\``;
}

function quoteStringLiteral(value: string): string {
  return `'${value.replaceAll("'", "''")}'`;
}

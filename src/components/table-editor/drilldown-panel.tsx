/**
 * Drizzle-Studio-style drill-down panel.
 *
 * Renders the rows of a foreign-key target table filtered to the
 * single value the user clicked on. Sits inside the same tab as a
 * stack frame — the parent `TableEditorPanel` owns the
 * `DrilldownEntry[]` stack and replaces the main grid with this
 * panel when the stack is non-empty.
 *
 * Each panel can push another frame onto the stack by following a
 * foreign key in its own rows, so the architecture composes
 * recursively: `users → orders → order_items → products…` Escape /
 * the breadcrumb's back arrow pops the topmost frame.
 *
 * SQL plumbing: the panel runs `SELECT * FROM "schema"."table" WHERE
 * "col" = 'value' LIMIT 100` via the engine-aware `run_query`
 * dispatch. Identifier + literal quoting is engine-specific (PG
 * uses `"`, ClickHouse uses backticks). Strings are escaped with
 * the doubled-quote convention both engines accept.
 */

import { IconArrowLeft, IconChevronRight, IconX } from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

import {
  type ColumnHeaderMeta,
  DataGrid,
  type ForeignKeyTarget,
} from "@/components/data-grid";
import { Button } from "@/components/ui/button";
import {
  type ColumnInfo,
  type DatabaseEngine,
  type TableStructure,
  tableStructureKey,
  useAppStore,
} from "@/lib/store";
import { errorToMessage, tauriInvoke } from "@/lib/tauri";

export type DrilldownEntry = {
  schema: string;
  table: string;
  filterColumn: string;
  filterValue: string;
};

export interface DrilldownPanelProps {
  connectionId: string;
  engine: DatabaseEngine;
  /** The current top-of-stack frame to render. */
  entry: DrilldownEntry;
  /**
   * Full path including the current entry — used for the breadcrumb
   * rendered above the grid. The originating table is the first
   * element; the current entry is the last.
   */
  breadcrumb: Array<{ schema: string; table: string }>;
  onPushDrilldown: (entry: DrilldownEntry) => void;
  /** Close the current (topmost) frame. */
  onPopDrilldown: () => void;
  /** Close every frame and return to the originating table. */
  onCloseAll: () => void;
}

const ROW_LIMIT = 100;

export function DrilldownPanel({
  connectionId,
  engine,
  entry,
  breadcrumb,
  onPushDrilldown,
  onPopDrilldown,
  onCloseAll,
}: DrilldownPanelProps) {
  const [rows, setRows] = useState<string[][]>([]);
  const [columns, setColumns] = useState<string[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const structureKey = tableStructureKey(
    connectionId,
    entry.schema,
    entry.table,
  );
  const structure = useAppStore((s) => s.tableStructure[structureKey]);
  const loadTableStructure = useAppStore((s) => s.loadTableStructure);

  // Ensure structure is loaded so the grid gets PK/FK column icons + the
  // FK targets needed for nested drill-downs.
  useEffect(() => {
    if (!structure) {
      void loadTableStructure(connectionId, entry.schema, entry.table);
    }
  }, [connectionId, entry.schema, entry.table, structure, loadTableStructure]);

  // Pull rows whenever the filter changes. Keyed by every field so a
  // user re-following the same FK on a different row refetches.
  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    const sql = buildFilteredSelectSql(engine, entry);
    tauriInvoke<{ columns: string[]; rows: string[][] }>("run_query", {
      payload: { connectionId, query: sql },
    })
      .then((result) => {
        if (cancelled) return;
        setColumns(result.columns);
        setRows(result.rows);
      })
      .catch((err) => {
        if (cancelled) return;
        setError(errorToMessage(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [
    connectionId,
    engine,
    entry.schema,
    entry.table,
    entry.filterColumn,
    entry.filterValue,
    entry,
  ]);

  const columnMetadata = useMemo<Array<ColumnHeaderMeta | undefined>>(
    () => buildColumnMetadataForDrilldown(columns, structure),
    [columns, structure],
  );

  // ESC pops the topmost frame.
  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onPopDrilldown();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onPopDrilldown]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-surface-app">
      <header className="flex shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-panel/60 px-3 py-1.5 text-[0.65rem]">
        <Button
          size="sm"
          variant="ghost"
          className="h-6 px-2 text-[0.65rem]"
          onClick={onPopDrilldown}
          aria-label="Back"
          title="Back (Esc)"
        >
          <IconArrowLeft className="size-3.5" />
        </Button>
        <nav
          aria-label="Drill-down path"
          className="flex flex-wrap items-center gap-1 text-text-muted"
        >
          {breadcrumb.map((crumb, idx) => {
            const isLast = idx === breadcrumb.length - 1;
            return (
              <span
                key={`${crumb.schema}.${crumb.table}.${idx}`}
                className="flex items-center gap-1"
              >
                <span
                  className={isLast ? "font-mono text-foreground" : "font-mono"}
                >
                  {crumb.schema}.{crumb.table}
                </span>
                {isLast ? null : (
                  <IconChevronRight className="size-3 shrink-0" />
                )}
              </span>
            );
          })}
        </nav>
        <span className="ml-2 truncate font-mono text-text-muted">
          where <span className="text-foreground">{entry.filterColumn}</span> ={" "}
          <span className="text-foreground">'{entry.filterValue}'</span>
        </span>
        <Button
          size="sm"
          variant="ghost"
          className="ml-auto h-6 px-2 text-[0.65rem]"
          onClick={onCloseAll}
          aria-label="Close drill-down"
          title="Close all drill-downs"
        >
          <IconX className="size-3.5" />
        </Button>
      </header>
      {error ? (
        <div
          role="alert"
          className="m-3 rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive"
        >
          {error}
        </div>
      ) : loading && rows.length === 0 ? (
        <div className="flex flex-1 items-center justify-center text-xs text-text-muted">
          Loading {entry.schema}.{entry.table}…
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-hidden">
          <DataGrid
            data={rows}
            columns={columns}
            columnMetadata={columnMetadata}
            onFollowForeignKey={(target, value) =>
              onPushDrilldown({
                schema: target.schema,
                table: target.table,
                filterColumn: target.column,
                filterValue: value,
              })
            }
            readOnly
            exportFilenameBase={`${entry.schema}-${entry.table}-${entry.filterColumn}-${entry.filterValue}`}
          />
        </div>
      )}
    </div>
  );
}

function buildColumnMetadataForDrilldown(
  columnNames: string[],
  structure: TableStructure | undefined,
): Array<ColumnHeaderMeta | undefined> {
  if (!structure) return columnNames.map(() => undefined);
  const fkColumns = new Set<string>();
  const fkStructured = new Map<string, ForeignKeyTarget>();
  for (const fk of structure.foreignKeys) {
    for (let i = 0; i < fk.columns.length; i++) {
      const col = fk.columns[i];
      const targetCol = fk.referencedColumns[i] ?? "?";
      fkColumns.add(col);
      fkStructured.set(col, {
        schema: fk.referencedSchema,
        table: fk.referencedTable,
        column: targetCol,
      });
    }
  }
  const indexed = new Set<string>();
  const unique = new Set<string>();
  for (const idx of structure.indexes) {
    for (const col of idx.columns) {
      indexed.add(col);
      if (idx.isUnique) unique.add(col);
    }
  }
  return columnNames.map((name) => {
    const info: ColumnInfo | undefined = structure.columns.find(
      (c) => c.name === name,
    );
    if (!info) return undefined;
    return {
      isPrimaryKey: info.isPrimaryKey,
      isForeignKey: fkColumns.has(name),
      isIndexed: indexed.has(name) && !info.isPrimaryKey,
      isUnique: unique.has(name) && !info.isPrimaryKey,
      notNull: !info.nullable,
      hasDefault: info.defaultValue !== null,
      dataType: info.dataType,
      derivationKind: info.derivationKind ?? null,
      foreignKeyTarget: fkStructured.get(name),
    };
  });
}

function buildFilteredSelectSql(
  engine: DatabaseEngine,
  entry: DrilldownEntry,
): string {
  const ident = engine === "ClickHouse" ? quoteIdentBacktick : quoteIdentDouble;
  const qualified = `${ident(entry.schema)}.${ident(entry.table)}`;
  const colExpr = ident(entry.filterColumn);
  const literal = quoteStringLiteral(entry.filterValue);
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

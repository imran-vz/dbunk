/**
 * Pure and quasi-pure helpers extracted from the relational-tables
 * slice's `commitTableEdits` and `deleteSelectedTableRows` actions.
 *
 * Rationale: the two dispatchers share four sequential phases —
 *   1. locate the loaded `TableDataState` for the requested table
 *      name (`findTableData`)
 *   2. validate connection + structure capabilities
 *      (`resolveEditContext`)
 *   3. transform the raw store edits into the backend payload shape
 *      (`buildEditPayload` / `buildDeleteRowsPayload`)
 *   4. await `commit_cell_edits` / `delete_rows`, then either return
 *      the synchronous result or drive the ClickHouse "queued"
 *      pending-mutations flow to completion
 *      (`pollMutationsToCompletion`)
 *
 * Phases 1, 2 and 4 are identical between the two actions; phases 3
 * differ only in payload shape. Pulling them out collapses the
 * cyclomatic / cognitive complexity of the dispatchers themselves and
 * makes each phase independently testable.
 *
 * Note on per-engine strategies: the *frontend* dispatchers do **not**
 * branch by engine — the engine-specific SQL lives in the Rust
 * backend behind a single `commit_cell_edits` / `delete_rows` Tauri
 * command. The only frontend-visible variation is the optional
 * "queued mutations" result that ClickHouse returns, which is already
 * gated by `pendingMutationsFromResult` returning `[]` for the
 * synchronous engines. That keeps these helpers engine-agnostic.
 */

import type { PendingMutation } from "@/lib/pending-mutations";
import { type MutationOutcome, trackMutations } from "@/lib/pending-mutations";
import { pickRowIdentity } from "@/lib/row-identity";
import {
  type BrowseIdentityKind,
  browseIdentityReadOnlyCopy,
  identityIsEditable,
} from "@/lib/table-browse";

import type {
  Connection,
  EditOutcome,
  TableDataState,
  TableRef,
  TableStructure,
} from "./types";
import { tableDataKey, tableStructureKey } from "./types";

export type CellEditPayload = {
  rowIndex: number;
  identity: Array<{ column: string; value: string | null }>;
  set: Array<{ column: string; value: string | null }>;
};

export type DeleteRowPayload = Array<{
  column: string;
  value: string | null;
}>;

/**
 * Locate the `TableDataState` entry whose `.table` matches the supplied
 * name. Returns the `[key, data]` pair so the caller can reuse the
 * key for `refreshTableData`.
 */
export const findTableData = (
  tableData: Record<string, TableDataState>,
  tableName: string,
): [string, TableDataState] | null => {
  const entry = Object.entries(tableData).find(
    ([, data]) => data.table === tableName,
  );
  return entry ?? null;
};

export type EditContextOk = {
  ok: true;
  data: TableDataState;
  dataKey: string;
  structure: TableStructure;
  identity: { columns: string[] };
  connection: Connection;
  columnIndexByName: Map<string, number>;
};

export type EditContextErr = { ok: false; reason: string };

export type EditContextResult = EditContextOk | EditContextErr;

/**
 * Run every up-front validation common to `commitTableEdits` and
 * `deleteSelectedTableRows`: table data loaded, structure loaded,
 * row identity selectable, connection exists, capability flag set,
 * identity columns present in the currently loaded data.
 *
 * `capability` is the structure capability key the action needs
 * (`canUpdateRows` vs `canDeleteRows`); the message includes the
 * supplied `action` label so the surfaced reason mirrors what the
 * original inline code wrote.
 */
export type EditDataSource = {
  connectionId: string;
  schema: string;
  table: string;
  columns: string[];
  rows: string[][];
  identityKind: BrowseIdentityKind;
  identityColumns: string[];
};

export const resolveEditContext = (params: {
  tableData: Record<string, TableDataState>;
  tableStructure: Record<string, TableStructure>;
  connections: Connection[];
  tableName?: string;
  ref?: TableRef;
  dataSource?: EditDataSource;
  capability: "canUpdateRows" | "canDeleteRows";
  action: "cell edits" | "row deletes";
}): EditContextResult => {
  const {
    tableData,
    tableStructure,
    connections,
    tableName,
    ref,
    dataSource,
    capability,
    action,
  } = params;

  const dataEntry = ((): [string, TableDataState] | null => {
    if (dataSource) {
      const key = tableDataKey(
        dataSource.connectionId,
        dataSource.schema,
        dataSource.table,
      );
      const data: TableDataState = {
        connectionId: dataSource.connectionId,
        schema: dataSource.schema,
        table: dataSource.table,
        columns: dataSource.columns,
        rows: dataSource.rows,
        page: 1,
        pageSize: dataSource.rows.length,
        runtimeMs: 0,
      };
      return [key, data];
    }
    if (ref) {
      const key = tableDataKey(ref.connectionId, ref.schema, ref.table);
      const data = tableData[key];
      return data ? [key, data] : null;
    }
    return findTableData(tableData, tableName ?? "");
  })();
  if (!dataEntry) {
    return {
      ok: false,
      reason:
        action === "cell edits"
          ? "Table data is not loaded; cannot commit edits."
          : "Table data is not loaded; cannot delete rows.",
    };
  }
  const [dataKey, data] = dataEntry;
  const structureKey = tableStructureKey(
    data.connectionId,
    data.schema,
    data.table,
  );
  const structure = tableStructure[structureKey];
  if (dataSource && !identityIsEditable(dataSource.identityKind)) {
    return {
      ok: false,
      reason: browseIdentityReadOnlyCopy(dataSource.identityKind),
    };
  }
  const identity = dataSource
    ? { columns: dataSource.identityColumns }
    : pickRowIdentity(structure);
  if (!identity) {
    return {
      ok: false,
      reason:
        "This table has no primary key or non-null unique index — it is read-only.",
    };
  }

  const connection = connections.find((c) => c.id === data.connectionId);
  if (!connection) {
    return { ok: false, reason: "Connection not found for this table." };
  }
  // For commit (canUpdateRows) the original behaviour required the
  // structure to be present (it dereferenced `structure.capabilities`
  // directly). For delete (canDeleteRows) the original behaviour
  // tolerated a missing structure (it guarded `structure &&`). We
  // preserve both shapes exactly.
  if (capability === "canUpdateRows") {
    if (!structure.capabilities.canUpdateRows) {
      return {
        ok: false,
        reason: `This table does not support ${action} on ${connection.engine}.`,
      };
    }
  } else if (structure && !structure.capabilities.canDeleteRows) {
    return {
      ok: false,
      reason: `This table does not support ${action} on ${connection.engine}.`,
    };
  }

  const columnIndexByName = new Map<string, number>();
  data.columns.forEach((name, index) => {
    columnIndexByName.set(name, index);
  });
  const identityMissing = identity.columns.filter(
    (col) => !columnIndexByName.has(col),
  );
  if (identityMissing.length > 0) {
    return {
      ok: false,
      reason: `Identity column(s) not present in loaded data: ${identityMissing.join(", ")}`,
    };
  }

  return {
    ok: true,
    data,
    dataKey,
    structure,
    identity,
    connection,
    columnIndexByName,
  };
};

/**
 * Pure transform: raw store edit map → ordered backend payload.
 *
 * Skips rows that vanished from the loaded data, columns whose new
 * value equals the original (no-op edit), and unknown column indices.
 * Returns an empty array when no row produces any actual change —
 * the caller treats that as a `noop`.
 */
export const buildEditPayload = (
  editsForTable: Record<number, Record<number, string>>,
  data: Pick<TableDataState, "columns" | "rows">,
  identity: { columns: string[] },
  columnIndexByName: Map<string, number>,
): CellEditPayload[] => {
  const payload: CellEditPayload[] = [];
  const sortedRowIndices = Object.keys(editsForTable)
    .map((k) => Number.parseInt(k, 10))
    .filter((n) => Number.isFinite(n))
    .sort((a, b) => a - b);

  for (const rowIndex of sortedRowIndices) {
    const row = data.rows[rowIndex];
    if (!row) continue;

    const colChanges = editsForTable[rowIndex] ?? {};
    const setEntries = buildSetEntries(colChanges, row, data.columns);
    if (setEntries.length === 0) continue;

    const identityEntries = identity.columns.map((col) => {
      // SAFETY: buildColumnIndex and chooseRowIdentity guarantee every identity column is indexed.
      const idx = columnIndexByName.get(col) as number;
      return { column: col, value: row[idx] ?? null };
    });

    payload.push({ rowIndex, identity: identityEntries, set: setEntries });
  }

  return payload;
};

const buildSetEntries = (
  colChanges: Record<number, string>,
  row: string[],
  columns: string[],
): Array<{ column: string; value: string | null }> => {
  const sortedColIndices = Object.keys(colChanges)
    .map((k) => Number.parseInt(k, 10))
    .filter((n) => Number.isFinite(n))
    .sort((a, b) => a - b);

  const out: Array<{ column: string; value: string | null }> = [];
  for (const colIndex of sortedColIndices) {
    const newValue = colChanges[colIndex];
    if (newValue === undefined) continue;
    const original = row[colIndex];
    if (newValue === original) continue;
    const columnName = columns[colIndex];
    if (!columnName) continue;
    out.push({ column: columnName, value: newValue });
  }
  return out;
};

/**
 * Pure transform: row indices → per-row identity tuples ready for the
 * `delete_rows` backend command. Skips indices whose row is missing
 * from the loaded page.
 */
export const buildDeleteRowsPayload = (
  rowIndices: number[],
  data: Pick<TableDataState, "rows">,
  identity: { columns: string[] },
  columnIndexByName: Map<string, number>,
): DeleteRowPayload[] => {
  const out: DeleteRowPayload[] = [];
  const sorted = [...rowIndices].sort((a, b) => a - b);
  for (const rowIndex of sorted) {
    const row = data.rows[rowIndex];
    if (!row) continue;
    out.push(
      identity.columns.map((col) => {
        // SAFETY: buildColumnIndex and chooseRowIdentity guarantee every identity column is indexed.
        const idx = columnIndexByName.get(col) as number;
        return { column: col, value: row[idx] ?? null };
      }),
    );
  }
  return out;
};

/**
 * Map the CH-internal `MutationOutcome` (terminal result of a Pending
 * Mutation batch) onto the caller-facing `EditOutcome` shape. Keeps
 * `MutationOutcome` CH-specific while every store action exposes
 * `EditOutcome` uniformly.
 */
export const mutationOutcomeToEditOutcome = (
  outcome: MutationOutcome,
  ctx: { startedAt: number },
): EditOutcome => {
  if (outcome.kind === "completed") {
    return { kind: "completed", runtimeMs: Date.now() - ctx.startedAt };
  }
  if (outcome.kind === "failed") {
    return {
      kind: "failed",
      reason: outcome.reason,
      mutationId: outcome.mutationId,
    };
  }
  return { kind: "timeout", remaining: outcome.remaining };
};

/**
 * Drive a ClickHouse-style "queued" batch to terminal completion and
 * fold the resulting `MutationOutcome` into the shared `EditOutcome`
 * shape. Caller owns lifecycle-slot bookkeeping and the optional
 * post-success `refreshTableData` — this helper is just the wait.
 */
export const pollMutationsToCompletion = async (
  pendingMutations: PendingMutation[],
): Promise<EditOutcome> => {
  const startedAt = Date.now();
  const outcome = await trackMutations(pendingMutations);
  return mutationOutcomeToEditOutcome(outcome, { startedAt });
};

export type StructureCommitContextOk = {
  ok: true;
  connection: Connection;
  ddlStructure: TableStructure | undefined;
  schema: string;
  table: string;
  connectionId: string;
};

export type StructureCommitContextResult =
  | StructureCommitContextOk
  | EditContextErr;

/**
 * Pure validation for the synchronous up-front phase of
 * `commitStructureChanges` — connection lookup and the structure
 * capability gate. Mirrors `resolveEditContext` for cell edits / row
 * deletes. The dispatcher is left with just the lifecycle, the
 * `tauriInvoke`, and the post-success refresh.
 *
 * Returns `null` for `ddlStructure` when the structure has not been
 * loaded yet; the caller still issues the DDL because the backend is
 * the source of truth for the schema (matches the pre-refactor
 * behaviour where the inline guard tolerated a missing structure).
 */
export const resolveStructureCommitContext = (params: {
  pending: ReadonlyArray<{ schema: string; table: string }>;
  key: string;
  connections: Connection[];
  tableStructure: Record<string, TableStructure>;
}): StructureCommitContextResult => {
  const { pending, key, connections, tableStructure } = params;
  const { schema, table } = pending[0];
  const connectionId = key.split("::")[0] ?? "";
  const connection = connections.find((c) => c.id === connectionId);
  if (!connection) {
    return { ok: false, reason: "Connection not found for this table." };
  }
  const ddlStructure = tableStructure[key];
  if (ddlStructure && !ddlStructure.capabilities.canAlterSchema) {
    return {
      ok: false,
      reason: `This table does not support schema edits on ${connection.engine}.`,
    };
  }
  return {
    ok: true,
    connection,
    ddlStructure,
    schema,
    table,
    connectionId,
  };
};

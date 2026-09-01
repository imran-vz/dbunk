import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import type {
  DatabaseEngine,
  PendingChange,
  PgObjectOp,
  PgReferentialAction,
  StructureChange,
} from "@/lib/store/types";

export type PendingStructureBatch =
  | { kind: "empty" }
  | { kind: "column"; changes: ColumnChangeKind[] }
  | { kind: "pg-op"; ops: PgObjectOp[] }
  | { kind: "invalid"; reason: string };

/** Narrow a pending list once so render and commit paths dispatch on the
 * batch representation instead of adding per-entry special cases. */
export const pendingStructureBatch = (
  pending: readonly PendingChange[],
): PendingStructureBatch => {
  const first = pending[0];
  if (!first) return { kind: "empty" };

  if (first.change.kind === "column") {
    const changes: ColumnChangeKind[] = [];
    for (const entry of pending) {
      if (entry.change.kind !== "column") {
        return {
          kind: "invalid",
          reason: "Pending structure changes contain mixed representations.",
        };
      }
      changes.push(entry.change.change);
    }
    return { kind: "column", changes };
  }

  const ops: PgObjectOp[] = [];
  for (const entry of pending) {
    if (entry.change.kind !== "pg-op") {
      return {
        kind: "invalid",
        reason: "Pending structure changes contain mixed representations.",
      };
    }
    ops.push(entry.change.op);
  }
  return { kind: "pg-op", ops };
};

/** Keep the store invariant at the append boundary instead of rediscovering
 * mixed pending batches in render, confirmation, and commit paths. */
export const assertStructureChangeCanAppend = (
  existing: readonly PendingChange[],
  next: StructureChange,
  engine: DatabaseEngine | undefined,
): void => {
  if (next.kind === "pg-op" && engine !== "PostgreSQL") {
    throw new Error(
      "PostgreSQL object operations require a PostgreSQL connection.",
    );
  }

  const batch = pendingStructureBatch(existing);
  if (batch.kind === "invalid") {
    throw new Error(batch.reason);
  }
  if (batch.kind !== "empty" && batch.kind !== next.kind) {
    throw new Error(
      `Cannot mix ${batch.kind} and ${next.kind} structure changes in one pending batch.`,
    );
  }
};

/** One vocabulary for FK referential actions across every surface that
 * builds `addForeignKey` ops (structure editor, specialized FK panel). */
export const PG_REFERENTIAL_ACTIONS: readonly {
  value: PgReferentialAction;
  label: string;
}[] = [
  { value: "no-action", label: "NO ACTION" },
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "set-null", label: "SET NULL" },
  { value: "set-default", label: "SET DEFAULT" },
];

/** Accepts either the tagged value or the SQL label; unknown input falls
 * back to the PostgreSQL default, NO ACTION. */
export const asPgReferentialAction = (value: string): PgReferentialAction =>
  PG_REFERENTIAL_ACTIONS.find(
    (option) => option.value === value || option.label === value,
  )?.value ?? "no-action";

const trimmedOrNull = (value: string): string | null => {
  const trimmed = value.trim();
  return trimmed === "" ? null : trimmed;
};

/** Normalizing constructors shared by the structure editor's forms and
 * the specialized editors, so the two surfaces cannot drift on how the
 * same user input becomes a typed op. */
export const buildCreateIndexOp = (input: {
  schema: string;
  table: string;
  name: string;
  unique: boolean;
  method: string;
  columnExpressions: string[];
  include: string[];
  wherePredicate: string;
  concurrently: boolean;
}): PgObjectOp => ({
  op: "createIndex",
  schema: input.schema,
  table: input.table,
  name: trimmedOrNull(input.name),
  unique: input.unique,
  method: input.method.trim(),
  columns: input.columnExpressions.map((expression) => ({
    expression,
    descending: false,
  })),
  include: input.include,
  wherePredicate: trimmedOrNull(input.wherePredicate),
  concurrently: input.concurrently,
});

export const buildAddForeignKeyOp = (input: {
  schema: string;
  table: string;
  name: string;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: PgReferentialAction;
  onDelete: PgReferentialAction;
  deferrable: boolean;
}): PgObjectOp => ({
  op: "addForeignKey",
  schema: input.schema,
  table: input.table,
  name: trimmedOrNull(input.name),
  columns: input.columns,
  referencedSchema: input.referencedSchema.trim() || input.schema,
  referencedTable: input.referencedTable.trim(),
  referencedColumns: input.referencedColumns,
  onUpdate: input.onUpdate,
  onDelete: input.onDelete,
  deferrable: input.deferrable,
  initiallyDeferred: input.deferrable,
  notValid: false,
});

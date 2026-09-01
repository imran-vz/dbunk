import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import type {
  DatabaseEngine,
  PendingChange,
  PgObjectOp,
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

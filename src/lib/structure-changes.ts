import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import type {
  DatabaseEngine,
  PendingChange,
  PgGrantee,
  PgObjectOp,
  PgObjectRef,
  PgParallelSafety,
  PgPolicyCommand,
  PgPrivilege,
  PgReferentialAction,
  PgTriggerEvent,
  PgTriggerLevel,
  PgTriggerTiming,
  PgVolatility,
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

export const PG_POLICY_COMMAND_OPTIONS = [
  { value: "all", label: "ALL" },
  { value: "select", label: "SELECT" },
  { value: "insert", label: "INSERT" },
  { value: "update", label: "UPDATE" },
  { value: "delete", label: "DELETE" },
] as const satisfies readonly { value: PgPolicyCommand; label: string }[];

export const PG_TRIGGER_EVENT_OPTIONS = [
  { value: "insert", label: "INSERT" },
  { value: "update", label: "UPDATE" },
  { value: "delete", label: "DELETE" },
  { value: "truncate", label: "TRUNCATE" },
] as const satisfies readonly {
  value: PgTriggerEvent["kind"];
  label: string;
}[];

/** Accepts either the tagged value or the SQL label; unknown input falls
 * back to the PostgreSQL default, NO ACTION. */
export const asPgReferentialAction = (value: string): PgReferentialAction =>
  PG_REFERENTIAL_ACTIONS.find(
    (option) => option.value === value || option.label === value,
  )?.value ?? "no-action";

export const asPgPolicyCommand = (value: string): PgPolicyCommand => {
  const command = PG_POLICY_COMMAND_OPTIONS.find(
    (option) => option.value === value.toLowerCase(),
  )?.value;
  if (!command)
    throw new Error(`Unsupported PostgreSQL policy command: ${value}`);
  return command;
};

export const asPgTriggerEvent = (value: string): PgTriggerEvent => {
  const kind = PG_TRIGGER_EVENT_OPTIONS.find(
    (option) => option.value === value.toLowerCase(),
  )?.value;
  if (!kind) throw new Error(`Unsupported PostgreSQL trigger event: ${value}`);
  return kind === "update" ? { kind, columns: [] } : { kind };
};

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

/** Preserve the explicit pseudo-role versus quoted-role choice while trimming
 * catalog/user input at the operation boundary. */
const normalizePgGrantee = (grantee: PgGrantee): PgGrantee =>
  grantee.kind === "public"
    ? grantee
    : { kind: "role", name: grantee.name.trim() };

/** Build the relation-scoped GRANT used by both table security surfaces. */
export const buildGrantOp = (input: {
  schema: string;
  table: string;
  grantee: PgGrantee;
  privileges: PgPrivilege[];
  allPrivileges?: boolean;
  withGrantOption: boolean;
}): PgObjectOp => ({
  op: "grantPrivileges",
  target: tableReference(input.schema, input.table),
  privileges: input.privileges,
  allPrivileges: input.allPrivileges ?? false,
  grantee: normalizePgGrantee(input.grantee),
  withGrantOption: input.withGrantOption,
});

/** Build the relation-scoped REVOKE used by the Structure tab. */
export const buildRevokeOp = (input: {
  schema: string;
  table: string;
  grantee: PgGrantee;
  privileges: PgPrivilege[];
  allPrivileges?: boolean;
  grantOptionFor?: boolean;
  cascade: boolean;
}): PgObjectOp => ({
  op: "revokePrivileges",
  target: tableReference(input.schema, input.table),
  privileges: input.privileges,
  allPrivileges: input.allPrivileges ?? false,
  grantee: normalizePgGrantee(input.grantee),
  grantOptionFor: input.grantOptionFor ?? false,
  cascade: input.cascade,
});

/** Ordered RLS/policy operations. Editing prepends the required drop. */
export const buildPolicyOps = (input: {
  schema: string;
  table: string;
  enabled?: boolean;
  force?: boolean | null;
  dropExisting?: string;
  name: string;
  permissive: boolean;
  command: PgPolicyCommand;
  roles: PgGrantee[];
  using: string;
  withCheck: string;
}): PgObjectOp[] => {
  const ops: PgObjectOp[] = [];
  if (input.enabled !== undefined) {
    ops.push({
      op: "setRowLevelSecurity",
      schema: input.schema,
      table: input.table,
      enabled: input.enabled,
      force: input.enabled ? (input.force ?? null) : false,
    });
  }
  if (input.dropExisting) {
    ops.push({
      op: "dropPolicy",
      schema: input.schema,
      table: input.table,
      name: input.dropExisting,
    });
  }
  ops.push({
    op: "createPolicy",
    schema: input.schema,
    table: input.table,
    name: input.name.trim(),
    permissive: input.permissive,
    command: input.command,
    roles: input.roles.map(normalizePgGrantee),
    using: input.command === "insert" ? null : trimmedOrNull(input.using),
    withCheck:
      input.command === "select" || input.command === "delete"
        ? null
        : trimmedOrNull(input.withCheck),
  });
  return ops;
};

/** Ordered trigger operations, optionally creating its function first. */
export const buildTriggerOps = (input: {
  schema: string;
  table: string;
  name: string;
  timing: PgTriggerTiming;
  events: PgTriggerEvent[];
  forEach: PgTriggerLevel;
  when: string;
  functionSchema: string;
  functionName: string;
  arguments?: string[];
  orReplace?: boolean;
  createFunction?: {
    arguments?: string;
    language: string;
    body: string;
    volatility?: PgVolatility;
    strict?: boolean;
    securityDefiner?: boolean;
    parallel?: PgParallelSafety | null;
  };
}): PgObjectOp[] => {
  const ops: PgObjectOp[] = [];
  if (input.createFunction) {
    ops.push({
      op: "createFunction",
      schema: input.functionSchema.trim(),
      name: input.functionName.trim(),
      orReplace: true,
      arguments: input.createFunction.arguments?.trim() ?? "",
      returns: "trigger",
      language: input.createFunction.language.trim(),
      body: input.createFunction.body,
      volatility: input.createFunction.volatility ?? "volatile",
      strict: input.createFunction.strict ?? false,
      securityDefiner: input.createFunction.securityDefiner ?? false,
      parallel: input.createFunction.parallel ?? null,
    });
  }
  ops.push({
    op: "createTrigger",
    schema: input.schema,
    table: input.table,
    name: input.name.trim(),
    timing: input.timing,
    events: input.events,
    forEach: input.forEach,
    when: trimmedOrNull(input.when),
    functionSchema: input.functionSchema.trim(),
    functionName: input.functionName.trim(),
    arguments: input.arguments ?? [],
    orReplace: input.orReplace ?? false,
  });
  return ops;
};

const tableReference = (schema: string, table: string): PgObjectRef => ({
  kind: "table",
  schema,
  name: table,
  identityArgs: null,
});

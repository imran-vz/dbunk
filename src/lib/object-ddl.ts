/* oxlint-disable anti-slop/no-unknown-parameters -- Tauri rejections are decoded into PgObjectError at this command boundary. */
import { canonicalPgObjectRefKey } from "@/lib/pg-object-ref";
import { confirmDdlStatements } from "@/lib/safety-confirmation";
import { decodePgObjectError } from "@/lib/store/pg-objects";
import type {
  Connection,
  DdlApplyResult,
  DdlPlanPreview,
  DdlStatementSummary,
  PgDropImpact,
  PgObjectError,
  PgObjectOp,
  PgObjectRef,
  PlannedStatement,
} from "@/lib/store/types";
import { isTauri, tauriInvoke } from "@/lib/tauri";

export { decodePgObjectError } from "@/lib/store/pg-objects";

export type ObjectDdlClientResult<T> =
  | { kind: "ok"; value: T }
  | { kind: "cancelled" }
  | { kind: "error"; error: PgObjectError };

type PreviewObjectDdlPayload = {
  connectionId: string;
  ops: PgObjectOp[];
};

type ApplyObjectDdlPayload = PreviewObjectDdlPayload & {
  confirmed: boolean;
};

type LoadPgDropImpactPayload = {
  connectionId: string;
  reference: PgObjectRef;
};

type ObjectDdlCommandPayload =
  | PreviewObjectDdlPayload
  | ApplyObjectDdlPayload
  | LoadPgDropImpactPayload;

const invokeObjectDdl = async <T>(
  command: string,
  payload: ObjectDdlCommandPayload,
): Promise<ObjectDdlClientResult<T>> => {
  if (!isTauri()) {
    return {
      kind: "error",
      error: { kind: "connection", message: "Tauri is unavailable." },
    };
  }
  try {
    return {
      kind: "ok",
      value: await tauriInvoke<T>(command, { payload }),
    };
  } catch (error) {
    return { kind: "error", error: decodePgObjectError(error) };
  }
};

export function previewObjectDdl(
  payload: PreviewObjectDdlPayload,
): Promise<ObjectDdlClientResult<DdlPlanPreview>> {
  return invokeObjectDdl("preview_object_ddl", payload);
}

export function applyObjectDdl(
  payload: ApplyObjectDdlPayload,
): Promise<ObjectDdlClientResult<DdlApplyResult>> {
  return invokeObjectDdl("apply_object_ddl", payload);
}

export function loadPgDropImpact(
  payload: LoadPgDropImpactPayload,
): Promise<ObjectDdlClientResult<PgDropImpact>> {
  return invokeObjectDdl("load_pg_drop_impact", payload);
}

/** Typed policy retry for object DDL. Legacy string refusal tags are not used. */
export async function applyObjectDdlWithSafetyConfirmation(
  payload: { connectionId: string; ops: PgObjectOp[] },
  connection: Connection | undefined,
  isConnectionCurrent: () => boolean = () => true,
): Promise<ObjectDdlClientResult<DdlApplyResult>> {
  const first = await applyObjectDdl({ ...payload, confirmed: false });
  if (
    first.kind !== "error" ||
    first.error.kind !== "policyNeedsConfirmation"
  ) {
    return first;
  }
  if (!connection) {
    return {
      kind: "error",
      error: { kind: "connection", message: "Connection not found." },
    };
  }
  const confirmed = await confirmDdlStatements(
    connection,
    first.error.statements,
  );
  if (!confirmed) return { kind: "cancelled" };
  if (!isConnectionCurrent()) {
    return {
      kind: "error",
      error: {
        kind: "connection",
        message: "Connection changed. Regenerate the DDL preview.",
      },
    };
  }
  return applyObjectDdl({ ...payload, confirmed: true });
}

type StatementLabel = Pick<PlannedStatement, "summary"> | DdlStatementSummary;

const failureLabel = (
  statementIndex: number | undefined,
  statements: readonly StatementLabel[],
): string => {
  if (statementIndex === undefined) return "DDL apply";
  const summary = statements[statementIndex]?.summary;
  return summary
    ? `Statement ${statementIndex + 1} (${summary})`
    : `Statement ${statementIndex + 1}`;
};

const appliedCopy = (appliedStatements: number): string =>
  appliedStatements > 0
    ? ` ${appliedStatements} earlier statement${appliedStatements === 1 ? " was" : "s were"} applied.`
    : "";

const residueCopy = (error: PgObjectError): string =>
  "residue" in error && error.residue
    ? ` An invalid index ${error.residue.schema}.${error.residue.name} was left behind. Drop it before retrying.`
    : "";

/** Exhaustive, statement-aware object DDL error copy. */
export function formatObjectDdlError(
  error: PgObjectError,
  statements: readonly StatementLabel[] = [],
): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return `Object DDL is not supported for ${error.engine}.`;
    case "objectNotFound":
      return `${error.reference.name} no longer exists.`;
    case "invalidOp":
      return `Operation ${error.opIndex + 1} is invalid: ${error.reason}`;
    case "policyBlocked":
      return error.reason;
    case "policyNeedsConfirmation":
      return "The safety policy requires confirmation.";
    case "connection":
      return error.message;
    case "lockTimeout":
      return `${failureLabel(error.statementIndex, statements)} timed out waiting for a database lock.${appliedCopy(error.appliedStatements)}${residueCopy(error)}`;
    case "database":
      return `${failureLabel(error.statementIndex, statements)} failed: ${error.message}${error.code ? ` [${error.code}]` : ""}.${appliedCopy(error.appliedStatements)}${residueCopy(error)}`;
  }
  error satisfies never;
}

export type ObjectDdlRefreshScope = {
  catalog: boolean;
  revalidateAllDescriptions: boolean;
  references: PgObjectRef[];
};

const tableRef = (schema: string, name: string): PgObjectRef => ({
  kind: "table",
  schema,
  name,
  identityArgs: null,
});

/** Determine exactly which object caches an operation can make stale. */
export function objectDdlRefreshScope(
  ops: readonly PgObjectOp[],
): ObjectDdlRefreshScope {
  let catalog = false;
  let revalidateAllDescriptions = false;
  const references = new Map<string, PgObjectRef>();
  const add = (reference: PgObjectRef) => {
    references.set(canonicalPgObjectRefKey(reference), reference);
  };

  for (const operation of ops) {
    switch (operation.op) {
      case "createSchema":
        catalog = true;
        add({
          kind: "schema",
          schema: null,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "renameObject":
        catalog = true;
        add(operation.reference);
        break;
      case "dropObject":
        catalog = true;
        revalidateAllDescriptions ||= operation.cascade;
        add(operation.reference);
        break;
      case "setComment":
        if (operation.target.kind === "object") {
          catalog = true;
          add(operation.target.reference);
        } else {
          add(tableRef(operation.target.schema, operation.target.table));
        }
        break;
      case "createView":
        catalog = true;
        add({
          kind: "view",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "createMaterializedView":
        catalog = true;
        add({
          kind: "materialized-view",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "createSequence":
        catalog = true;
        add({
          kind: "sequence",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "createEnum":
        catalog = true;
        add({
          kind: "type",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "createTable":
        catalog = true;
        add(tableRef(operation.schema, operation.name));
        break;
      case "createFunction":
        catalog = true;
        revalidateAllDescriptions ||= operation.orReplace;
        break;
      case "createProcedure":
        catalog = true;
        revalidateAllDescriptions ||= operation.orReplace;
        break;
      case "alterSequence":
        add({
          kind: "sequence",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "addEnumValue":
      case "renameEnumValue":
        add({
          kind: "type",
          schema: operation.schema,
          name: operation.name,
          identityArgs: null,
        });
        break;
      case "addColumn":
      case "dropColumn":
      case "renameColumn":
      case "alterColumnType":
      case "setColumnNullable":
      case "setColumnDefault":
      case "addPrimaryKey":
      case "addUnique":
      case "addForeignKey":
      case "addCheck":
      case "dropConstraint":
      case "createIndex":
      case "createTrigger":
      case "dropTrigger":
      case "setTriggerEnabled":
      case "setRowLevelSecurity":
      case "createPolicy":
      case "dropPolicy":
        add(tableRef(operation.schema, operation.table));
        break;
      case "dropIndex":
        break;
      case "grantPrivileges":
      case "revokePrivileges":
        add(operation.target);
        break;
    }
  }
  return {
    catalog,
    revalidateAllDescriptions,
    references: [...references.values()],
  };
}

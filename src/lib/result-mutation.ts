import {
  decodeDatabaseError,
  decodeTimeout,
  decodeTlsFailed,
  isNullableNumber,
  isNullableString,
  isRecord,
} from "@/lib/decode-transport-error";
import {
  decodeStatementSummaries,
  type StatementClassSummary,
} from "@/lib/safety-policy";
/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters, anti-slop/no-unsafe-dictionary-type -- Tauri errors cross this boundary as unstructured values. */
import type { DatabaseEngine, TlsFailureKind } from "@/lib/store/types";

export type AnalyzeSource =
  | { kind: "statement"; sql: string }
  | { kind: "relation"; schema: string; table: string };

export type AnalyzeResultSetPayload = {
  connectionId: string;
  tabId: string;
  requestId: number;
  source: AnalyzeSource;
  refreshStructure: boolean;
};

export type ColumnOrigin =
  | {
      kind: "table";
      schema: string;
      table: string;
      column: string;
      attnum: number;
    }
  | { kind: "expression" };

export type ColumnWritability =
  | { kind: "writable" }
  | { kind: "generated" }
  | { kind: "identityAlways" }
  | { kind: "systemColumn" };

export type AnalyzedColumn = {
  name: string;
  origin: ColumnOrigin;
  castType: string;
  nullable: boolean;
  writability: ColumnWritability;
};

export type MutationIdentityKind =
  | "primaryKey"
  | "uniqueIndex"
  | "virtualKey"
  | "ctidFallback"
  | "none";

export type MutationIdentity = {
  kind: MutationIdentityKind;
  columns: string[];
};

export type CapabilityReason =
  | "notAnalyzable"
  | "noIdentity"
  | "identityNotProjected"
  | "multipleOriginTables"
  | "noWritableColumns"
  | "ctidInsertUnsupported"
  | "invalidVirtualKey";

export type CapabilityVerdict = {
  allowed: boolean;
  reason?: CapabilityReason;
};

export type AnalyzedTable = {
  schema: string;
  table: string;
  identity: MutationIdentity;
  identityProjected: boolean;
  identityProjectionIndexes: number[];
  updatable: CapabilityVerdict;
  deletable: CapabilityVerdict;
  insertable: CapabilityVerdict;
};

export type DatabaseNotAnalyzableReason = {
  kind: "database";
  code: string | null;
  message: string;
  severity: string | null;
  position: number | null;
};

export type NotAnalyzableReason =
  | { kind: "multiStatement" }
  | { kind: "noProjectedColumns" }
  | { kind: "noTableOrigins" }
  | { kind: "possibleTempShadowing" }
  | DatabaseNotAnalyzableReason;

export type AnalysisStatement =
  | { kind: "analyzed" }
  | { kind: "notAnalyzable"; reason: NotAnalyzableReason };

export type AnalyzeResultSetResult = {
  requestId: number;
  analysisId: number;
  columns: AnalyzedColumn[];
  tables: AnalyzedTable[];
  statement: AnalysisStatement;
};

export type MutationTable = { schema: string; table: string };

export type MutationValue = { column: string; value: string | null };

export type MutationOp =
  | {
      kind: "update";
      table: MutationTable;
      identity: MutationValue[];
      guards: MutationValue[];
      set: MutationValue[];
    }
  | {
      kind: "delete";
      table: MutationTable;
      identity: MutationValue[];
      guards: MutationValue[];
    }
  | { kind: "insert"; table: MutationTable; values: MutationValue[] };

export type MutationPlan = { operations: MutationOp[] };

export type PreviewResultMutationsPayload = {
  connectionId: string;
  tabId: string;
  analysisId: number;
  plan: MutationPlan;
};

export type ApplyResultMutationsPayload = {
  connectionId: string;
  tabId: string;
  requestId: number;
  analysisId: number;
  plan: MutationPlan;
  confirmed?: boolean;
};

export type CancelResultMutationPayload = {
  connectionId: string;
  tabId: string;
};

export type CloseResultMutationPayload = { connectionId: string };

export type LoadVirtualKeyPayload = {
  connectionId: string;
  schema: string;
  table: string;
};

export type SaveVirtualKeyPayload = LoadVirtualKeyPayload & {
  columns: string[];
};

export type ClearVirtualKeyPayload = LoadVirtualKeyPayload;

export type VirtualKey = { version: number; columns: string[] };

export type DmlParam = { kind: "text"; value: string | null };

export type PreviewStatement = {
  opIndex: number;
  sql: string;
  params: DmlParam[];
};

export type PreviewResult = { statements: PreviewStatement[] };

export type AppliedOperation = { opIndex: number; rowsAffected: number };

export type ApplyResult = {
  operations: AppliedOperation[];
  runtimeMs: number;
};

export type CancelResultMutationResult = { cancelRequested: boolean };

export type InvalidPlanReason =
  | "emptySet"
  | "emptyIdentity"
  | "nullKeyedIdentity"
  | "missingGuard"
  | "identityMismatch"
  | "tableMismatch"
  | "duplicateColumn"
  | "generatedColumn"
  | "identityAlwaysColumn"
  | "systemColumn"
  | "noIdentity"
  | "multipleOriginTables";

export type ResultMutationError =
  | { kind: "unsupportedEngine" }
  | { kind: "notAnalyzable"; reason: NotAnalyzableReason }
  | { kind: "unknownColumn"; column: string }
  | { kind: "invalidPlan"; reason: InvalidPlanReason }
  | { kind: "analysisExpired" }
  | { kind: "conflict"; opIndex: number }
  | { kind: "identityNotUnique"; opIndex: number }
  | { kind: "lockTimeout"; opIndex: number }
  | { kind: "busy" }
  | { kind: "superseded" }
  | { kind: "cancelled" }
  | { kind: "connectionClosing" }
  | { kind: "connectionLost" }
  | { kind: "tlsFailed"; tlsKind: TlsFailureKind; message: string }
  | { kind: "policyBlocked"; reason: string }
  | { kind: "policyNeedsConfirmation"; statements: StatementClassSummary[] }
  | { kind: "timeout"; operation: string }
  | {
      kind: "database";
      code: string | null;
      message: string;
      severity: string | null;
      position: number | null;
      opIndex?: number;
    };

const decodeInvalidPlanReason = (value: unknown): InvalidPlanReason | null => {
  switch (value) {
    case "emptySet":
    case "emptyIdentity":
    case "nullKeyedIdentity":
    case "missingGuard":
    case "identityMismatch":
    case "tableMismatch":
    case "duplicateColumn":
    case "generatedColumn":
    case "identityAlwaysColumn":
    case "systemColumn":
    case "noIdentity":
    case "multipleOriginTables":
      return value;
    default:
      return null;
  }
};

const decodeNotAnalyzableReason = (
  value: unknown,
): NotAnalyzableReason | null => {
  if (!isRecord(value) || typeof value.kind !== "string") return null;
  switch (value.kind) {
    case "multiStatement":
    case "noProjectedColumns":
    case "noTableOrigins":
    case "possibleTempShadowing":
      return { kind: value.kind };
    case "database":
      if (
        !isNullableString(value.code) ||
        typeof value.message !== "string" ||
        !isNullableString(value.severity) ||
        !isNullableNumber(value.position)
      ) {
        return null;
      }
      return {
        kind: "database",
        code: value.code,
        message: value.message,
        severity: value.severity,
        position: value.position,
      };
    default:
      return null;
  }
};

/** Decode, and structurally validate, an untrusted Tauri command rejection. */
export function decodeResultMutationError(error: unknown): ResultMutationError {
  if (!isRecord(error) || typeof error.kind !== "string") {
    return { kind: "connectionLost" };
  }
  switch (error.kind) {
    case "unsupportedEngine":
    case "analysisExpired":
    case "busy":
    case "superseded":
    case "cancelled":
    case "connectionClosing":
    case "connectionLost":
      return { kind: error.kind };
    case "policyBlocked":
      return typeof error.reason === "string"
        ? { kind: "policyBlocked", reason: error.reason }
        : { kind: "connectionLost" };
    case "policyNeedsConfirmation": {
      const statements = decodeStatementSummaries(error.statements);
      return statements
        ? { kind: "policyNeedsConfirmation", statements }
        : { kind: "connectionLost" };
    }
    case "notAnalyzable": {
      const reason = decodeNotAnalyzableReason(error.reason);
      return reason
        ? { kind: "notAnalyzable", reason }
        : { kind: "connectionLost" };
    }
    case "unknownColumn":
      return typeof error.column === "string"
        ? { kind: "unknownColumn", column: error.column }
        : { kind: "connectionLost" };
    case "invalidPlan": {
      const reason = decodeInvalidPlanReason(error.reason);
      return reason
        ? { kind: "invalidPlan", reason }
        : { kind: "connectionLost" };
    }
    case "conflict":
    case "identityNotUnique":
    case "lockTimeout":
      return typeof error.opIndex === "number"
        ? { kind: error.kind, opIndex: error.opIndex }
        : { kind: "connectionLost" };
    case "tlsFailed":
      return decodeTlsFailed(error) ?? { kind: "connectionLost" };
    case "timeout":
      return decodeTimeout(error) ?? { kind: "connectionLost" };
    case "database": {
      const decoded = decodeDatabaseError(error);
      if (
        !decoded ||
        (error.opIndex !== undefined && typeof error.opIndex !== "number")
      ) {
        return { kind: "connectionLost" };
      }
      return typeof error.opIndex === "number"
        ? { ...decoded, opIndex: error.opIndex }
        : decoded;
    }
    default:
      return { kind: "connectionLost" };
  }
}

/** The single activation and rollback seam for staged result mutations. */
export function supportsResultMutations(engine: DatabaseEngine): boolean {
  return engine === "PostgreSQL";
}

export function usesProjectedRowGuards(kind: MutationIdentityKind): boolean {
  return kind === "virtualKey" || kind === "ctidFallback";
}

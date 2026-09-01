/* oxlint-disable anti-slop/no-runtime-typeof, anti-slop/no-unknown-parameters -- Tauri rejections are decoded at this slice boundary before entering typed store state. */
/**
 * PostgreSQL Object Catalog slice.
 *
 * Catalog and description reads share a per-connection generation. Disconnect
 * bumps that generation before any asynchronous teardown, so a late response
 * from an earlier connection lifetime cannot repopulate either cache.
 */

import type { StateCreator } from "zustand";

import {
  isNullableNumber,
  isNullableString,
  isRecord,
} from "@/lib/decode-transport-error";
import {
  pgObjectDescriptionKey,
  validatePgObjectRef,
} from "@/lib/pg-object-ref";
import {
  normalizePgObjectDescription,
  type PgObjectDescriptionPayload,
} from "@/lib/table-structure-contract";
import { errorToMessage, tauriInvoke } from "@/lib/tauri";

import type {
  AppStoreState,
  DdlResidue,
  DdlStatementSummary,
  PgObjectCatalog,
  PgObjectDescription,
  PgObjectError,
  PgObjectRef,
  SchemaExplorer,
} from "./types";

const decodeDdlResidue = (value: unknown): DdlResidue | undefined => {
  if (
    !isRecord(value) ||
    value.kind !== "invalidIndex" ||
    typeof value.schema !== "string" ||
    typeof value.name !== "string"
  ) {
    return undefined;
  }
  return { kind: "invalidIndex", schema: value.schema, name: value.name };
};

const decodeStatementSummary = (value: unknown): DdlStatementSummary | null => {
  if (
    !isRecord(value) ||
    typeof value.index !== "number" ||
    typeof value.summary !== "string" ||
    typeof value.destructive !== "boolean" ||
    typeof value.transactional !== "boolean"
  ) {
    return null;
  }
  return {
    index: value.index,
    summary: value.summary,
    destructive: value.destructive,
    transactional: value.transactional,
  };
};

/** Decode the complete Plan 013 error union at the Tauri boundary. */
export function decodePgObjectError(error: unknown): PgObjectError {
  if (!isRecord(error) || typeof error.kind !== "string") {
    return { kind: "connection", message: errorToMessage(error) };
  }
  switch (error.kind) {
    case "unsupportedEngine":
      return typeof error.engine === "string"
        ? { kind: "unsupportedEngine", engine: error.engine }
        : { kind: "connection", message: errorToMessage(error) };
    case "objectNotFound": {
      const reference = validatePgObjectRef(error.reference);
      return reference
        ? { kind: "objectNotFound", reference }
        : { kind: "connection", message: errorToMessage(error) };
    }
    case "invalidOp":
      return typeof error.opIndex === "number" &&
        typeof error.reason === "string"
        ? {
            kind: "invalidOp",
            opIndex: error.opIndex,
            reason: error.reason,
          }
        : { kind: "connection", message: errorToMessage(error) };
    case "policyBlocked":
      return typeof error.reason === "string"
        ? { kind: "policyBlocked", reason: error.reason }
        : { kind: "connection", message: errorToMessage(error) };
    case "policyNeedsConfirmation": {
      if (!Array.isArray(error.statements)) {
        return { kind: "connection", message: errorToMessage(error) };
      }
      const statements = error.statements.map(decodeStatementSummary);
      return statements.every(
        (statement): statement is DdlStatementSummary => statement !== null,
      )
        ? { kind: "policyNeedsConfirmation", statements }
        : { kind: "connection", message: errorToMessage(error) };
    }
    case "connection":
      return typeof error.message === "string"
        ? { kind: "connection", message: error.message }
        : { kind: "connection", message: errorToMessage(error) };
    case "lockTimeout": {
      if (
        typeof error.statementIndex !== "number" ||
        typeof error.appliedStatements !== "number"
      ) {
        return { kind: "connection", message: errorToMessage(error) };
      }
      const residue = decodeDdlResidue(error.residue);
      const decoded: PgObjectError = {
        kind: "lockTimeout",
        statementIndex: error.statementIndex,
        appliedStatements: error.appliedStatements,
      };
      if (residue) decoded.residue = residue;
      return decoded;
    }
    case "database": {
      if (
        !isNullableString(error.code) ||
        typeof error.message !== "string" ||
        !isNullableNumber(error.position) ||
        typeof error.appliedStatements !== "number" ||
        (error.statementIndex !== undefined &&
          typeof error.statementIndex !== "number")
      ) {
        return { kind: "connection", message: errorToMessage(error) };
      }
      const residue = decodeDdlResidue(error.residue);
      const decoded: PgObjectError = {
        kind: "database",
        code: error.code,
        message: error.message,
        position: error.position,
        appliedStatements: error.appliedStatements,
      };
      if (error.statementIndex !== undefined) {
        decoded.statementIndex = error.statementIndex;
      }
      if (residue) decoded.residue = residue;
      return decoded;
    }
    default:
      return { kind: "connection", message: errorToMessage(error) };
  }
}

/** Concise catalog-load copy. Step 4 adds statement-aware DDL formatting. */
export function formatPgCatalogError(error: PgObjectError): string {
  switch (error.kind) {
    case "unsupportedEngine":
      return `Object catalog is not supported for ${error.engine}.`;
    case "objectNotFound":
      return `${error.reference.name} no longer exists.`;
    case "invalidOp":
      return `Object request ${error.opIndex + 1} is invalid: ${error.reason}`;
    case "policyBlocked":
      return error.reason;
    case "policyNeedsConfirmation":
      return "This object request needs confirmation.";
    case "connection":
      return error.message;
    case "lockTimeout":
      return "The object request timed out waiting for a database lock.";
    case "database":
      return error.message;
  }
}

const displayRoutineName = (entry: {
  name: string;
  identityArgs?: string;
}): string =>
  entry.identityArgs === undefined
    ? entry.name
    : `${entry.name}(${entry.identityArgs})`;

/**
 * Derive the legacy explorer once when a catalog loads. Routine display names
 * remain compatible with completion and palette consumers, while the typed
 * catalog retains overload-safe identity separately.
 */
export function catalogToSchemaExplorer(
  catalog: PgObjectCatalog,
): SchemaExplorer[] {
  return catalog.schemas.map((schema) => ({
    name: schema.name,
    tables: schema.tables.map((entry) => entry.name),
    views: schema.views.map((entry) => entry.name),
    materializedViews: schema.materializedViews.map((entry) => entry.name),
    sequences: schema.sequences.map((entry) => entry.name),
    foreignTables: schema.foreignTables.map((entry) => entry.name),
    functions: schema.functions.map(displayRoutineName),
    procedures: schema.procedures.map(displayRoutineName),
    aggregateFunctions: schema.aggregates.map(displayRoutineName),
    types: schema.types.map((entry) => entry.name),
    domains: schema.domains.map((entry) => entry.name),
    extensions: schema.extensions.map((entry) => entry.name),
  }));
}

export {
  canonicalPgObjectRefKey,
  pgObjectDescriptionKey,
} from "@/lib/pg-object-ref";

export type PgObjectLoadStatus = "idle" | "loading" | "ready" | "error";

export type PgObjectCatalogState = {
  status: PgObjectLoadStatus;
  catalog?: PgObjectCatalog;
  error?: PgObjectError;
  generation: number;
};

export type PgObjectDescriptionState = {
  status: PgObjectLoadStatus;
  description?: PgObjectDescription;
  error?: PgObjectError;
  generation: number;
};

export type PgObjectLoadResult = "ready" | "error" | "stale";

export type PgObjectsSlice = {
  pgObjectCatalog: Record<string, PgObjectCatalogState>;
  pgObjectDescriptions: Record<string, PgObjectDescriptionState>;
  /** Latest catalog request admitted for each connection lifetime. */
  pgObjectCatalogRequestIds: Record<string, number>;
  /** Latest description request admitted for each canonical object key. */
  pgObjectDescriptionRequestIds: Record<string, number>;
  /** One in-flight DDL apply per connection, surviving viewer unmounts. */
  pgObjectDdlApplying: Record<string, true>;
  /** Any applied DDL invalidates every preview produced before this version. */
  pgObjectDdlVersion: number;
  loadPgObjectCatalog: (
    connectionId: string,
    expectedGeneration?: number,
  ) => Promise<PgObjectLoadResult>;
  loadPgObjectDescription: (
    connectionId: string,
    reference: PgObjectRef,
    expectedGeneration?: number,
  ) => Promise<PgObjectLoadResult>;
  beginPgObjectDdlApply: (key: string) => boolean;
  endPgObjectDdlApply: (key: string) => void;
  markPgObjectDdlApplied: () => void;
  dropPgObjectDescriptionsForSchema: (
    connectionId: string,
    schema: string,
  ) => void;
  dropPgObjectCachesForConnection: (connectionId: string) => void;
};

export { pgObjectDdlApplyKey } from "@/lib/pg-object-ref";

const generationFor = (state: AppStoreState, connectionId: string): number =>
  state.pgObjectCatalog[connectionId]?.generation ?? 0;

const descriptionKeyTargetsSchema = (
  key: string,
  connectionId: string,
  schema: string,
): boolean => {
  const prefix = `${connectionId}:`;
  if (!key.startsWith(prefix)) return false;
  try {
    const identity: unknown = JSON.parse(key.slice(prefix.length));
    if (!Array.isArray(identity)) return false;
    const [kind, scopedSchema, name] = identity;
    return scopedSchema === schema || (kind === "schema" && name === schema);
  } catch {
    return false;
  }
};

export const createPgObjectsSlice: StateCreator<
  AppStoreState,
  [],
  [],
  PgObjectsSlice
> = (set, get) => ({
  pgObjectCatalog: {},
  pgObjectDescriptions: {},
  pgObjectCatalogRequestIds: {},
  pgObjectDescriptionRequestIds: {},
  pgObjectDdlApplying: {},
  pgObjectDdlVersion: 0,

  beginPgObjectDdlApply: (key) => {
    let acquired = false;
    set((state) => {
      if (state.pgObjectDdlApplying[key]) return {};
      acquired = true;
      return {
        pgObjectDdlApplying: { ...state.pgObjectDdlApplying, [key]: true },
      };
    });
    return acquired;
  },

  endPgObjectDdlApply: (key) => {
    set((state) => {
      const { [key]: _finished, ...remaining } = state.pgObjectDdlApplying;
      return { pgObjectDdlApplying: remaining };
    });
  },

  markPgObjectDdlApplied: () => {
    set((state) => ({ pgObjectDdlVersion: state.pgObjectDdlVersion + 1 }));
  },

  loadPgObjectCatalog: async (connectionId, expectedGeneration) => {
    const generation = expectedGeneration ?? generationFor(get(), connectionId);
    if (generationFor(get(), connectionId) !== generation) {
      return "stale";
    }
    const requestId = (get().pgObjectCatalogRequestIds[connectionId] ?? 0) + 1;
    set((state) => ({
      pgObjectCatalogRequestIds: {
        ...state.pgObjectCatalogRequestIds,
        [connectionId]: requestId,
      },
      pgObjectCatalog: {
        ...state.pgObjectCatalog,
        [connectionId]: { status: "loading", generation },
      },
    }));
    try {
      const catalog = await tauriInvoke<PgObjectCatalog>(
        "load_pg_object_catalog",
        { payload: { connectionId } },
      );
      if (
        generationFor(get(), connectionId) !== generation ||
        get().pgObjectCatalogRequestIds[connectionId] !== requestId
      ) {
        return "stale";
      }
      const explorer = catalogToSchemaExplorer(catalog);
      get().setSchemaExplorerForConnection(connectionId, explorer);
      set((state) => ({
        pgObjectCatalog: {
          ...state.pgObjectCatalog,
          [connectionId]: { status: "ready", catalog, generation },
        },
      }));
      return "ready";
    } catch (error) {
      if (
        generationFor(get(), connectionId) !== generation ||
        get().pgObjectCatalogRequestIds[connectionId] !== requestId
      ) {
        return "stale";
      }
      const decoded = decodePgObjectError(error);
      set((state) => ({
        pgObjectCatalog: {
          ...state.pgObjectCatalog,
          [connectionId]: { status: "error", error: decoded, generation },
        },
      }));
      return "error";
    }
  },

  loadPgObjectDescription: async (
    connectionId,
    reference,
    expectedGeneration,
  ) => {
    const generation = expectedGeneration ?? generationFor(get(), connectionId);
    if (generationFor(get(), connectionId) !== generation) {
      return "stale";
    }
    const key = pgObjectDescriptionKey(connectionId, reference);
    const requestId = (get().pgObjectDescriptionRequestIds[key] ?? 0) + 1;
    set((state) => {
      const current = state.pgObjectDescriptions[key];
      const loading: PgObjectDescriptionState = {
        status: "loading",
        generation,
      };
      if (current?.generation === generation && current.description) {
        loading.description = current.description;
      }
      return {
        pgObjectDescriptionRequestIds: {
          ...state.pgObjectDescriptionRequestIds,
          [key]: requestId,
        },
        pgObjectDescriptions: {
          ...state.pgObjectDescriptions,
          [key]: loading,
        },
      };
    });
    try {
      const description = normalizePgObjectDescription(
        await tauriInvoke<PgObjectDescriptionPayload>("describe_pg_object", {
          payload: { connectionId, reference },
        }),
      );
      if (
        generationFor(get(), connectionId) !== generation ||
        get().pgObjectDescriptionRequestIds[key] !== requestId
      ) {
        return "stale";
      }
      set((state) => ({
        pgObjectDescriptions: {
          ...state.pgObjectDescriptions,
          [key]: { status: "ready", description, generation },
        },
      }));
      return "ready";
    } catch (error) {
      if (
        generationFor(get(), connectionId) !== generation ||
        get().pgObjectDescriptionRequestIds[key] !== requestId
      ) {
        return "stale";
      }
      const decoded = decodePgObjectError(error);
      set((state) => ({
        pgObjectDescriptions: {
          ...state.pgObjectDescriptions,
          [key]: { status: "error", error: decoded, generation },
        },
      }));
      return "error";
    }
  },

  dropPgObjectDescriptionsForSchema: (connectionId, schema) => {
    set((state) => {
      const descriptions: Record<string, PgObjectDescriptionState> = {};
      const requestIds = { ...state.pgObjectDescriptionRequestIds };
      for (const [key, value] of Object.entries(state.pgObjectDescriptions)) {
        if (descriptionKeyTargetsSchema(key, connectionId, schema)) {
          requestIds[key] = (requestIds[key] ?? 0) + 1;
        } else {
          descriptions[key] = value;
        }
      }
      return {
        pgObjectDescriptions: descriptions,
        pgObjectDescriptionRequestIds: requestIds,
      };
    });
  },

  dropPgObjectCachesForConnection: (connectionId) => {
    set((state) => {
      const prefix = `${connectionId}:`;
      const descriptions: Record<string, PgObjectDescriptionState> = {};
      const descriptionRequestIds = {
        ...state.pgObjectDescriptionRequestIds,
      };
      for (const [key, value] of Object.entries(state.pgObjectDescriptions)) {
        if (!key.startsWith(prefix)) {
          descriptions[key] = value;
        } else {
          descriptionRequestIds[key] = (descriptionRequestIds[key] ?? 0) + 1;
        }
      }
      return {
        pgObjectCatalogRequestIds: {
          ...state.pgObjectCatalogRequestIds,
          [connectionId]:
            (state.pgObjectCatalogRequestIds[connectionId] ?? 0) + 1,
        },
        pgObjectCatalog: {
          ...state.pgObjectCatalog,
          [connectionId]: {
            status: "idle",
            generation: generationFor(state, connectionId) + 1,
          },
        },
        pgObjectDescriptions: descriptions,
        pgObjectDescriptionRequestIds: descriptionRequestIds,
      };
    });
  },
});

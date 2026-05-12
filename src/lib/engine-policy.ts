/**
 * Engine UI policy — frontend mirror to backend Engine Dispatch.
 *
 * The backend's `dispatch/mod.rs` is the single place that knows
 * "what does this operation mean on each engine." This module is the
 * frontend analogue: one place that knows "how does this engine
 * appear in the UI" — connection-form shape, structure-view labels,
 * stats-card semantics, and the copy we use when a concept doesn't
 * apply.
 *
 * ## Shape (post-ADR-0008)
 *
 * `EnginePolicy` is a discriminated union on `storageClass`. A
 * shared base — `{ engine, connectionForm }` — sits outside the
 * union because the connection form is the only surface that
 * renders every engine through a single component (it carries
 * per-engine feature flags inside). Everything else is
 * storage-class-specific.
 *
 * Consumers narrow via `relationalPolicy()` / `keyvaluePolicy()`
 * helpers at the entry points of storage-class-specific components.
 * Deep call sites don't re-narrow.
 *
 * `TypeScript`'s `Record<DatabaseEngine, EnginePolicy>` enforces
 * exhaustiveness — a new engine variant won't compile until its
 * policy is filled in.
 *
 * ## Scope (deliberately narrow)
 *
 * Engine-level decisions only. Per-table mutation policy (can edit
 * cells? row identity columns?) lives on the
 * `TableStructure.capabilities` flag plus `pickRowIdentity` for
 * relational engines. Per-key write gating for keyvalue engines is
 * computed at the editor render site from the connection's
 * auto-read-only state plus the value type (ADR-0009).
 *
 * ## Adding an engine
 *
 * Add the variant to `DatabaseEngine` in `src/lib/store.ts`, then add
 * a `POLICIES` entry here matching the correct union arm. The
 * compiler will refuse to build until both halves agree.
 */

import type { DatabaseEngine, StorageClass } from "@/lib/store";

export type ConnectionFormPolicy = {
  /**
   * Whether the connection form needs host/port/user/password
   * inputs. SQLite is a file-path-only engine; the form collapses
   * to just the database-file input.
   */
  requiresHostAndAuth: boolean;
  /**
   * ClickHouse-only: surface the HTTPS toggle and URL-path input
   * under Advanced Options.
   */
  showClickHouseHttp: boolean;
  /**
   * Redis-only: surface the TLS toggle and verify-cert toggle
   * under Advanced Options.
   */
  showRedisTls: boolean;
  /**
   * Redis-only: surface the DB number input (0–15) on the form.
   */
  showRedisDbNumber: boolean;
  /**
   * Default port placeholder when the user hasn't typed one.
   */
  defaultPort: number;
};

// ---------------------------------------------------------------------------
// Relational engine policy
// ---------------------------------------------------------------------------

export type StructureLabels = {
  primaryKey: string;
  primaryKeyBadge: string;
  indexes: string;
  noPrimaryKey: string;
  noIndexes: string;
};

export type RelationalEnginePolicy = {
  storageClass: "relational";
  /**
   * `exact` — backend reports a precise row count cheaply (CH).
   * `estimate` — planner estimate (PG `pg_class.reltuples`); the
   * stats card adds a "(≈)" suffix.
   */
  rowCountKind: "exact" | "estimate";
  /**
   * Whether the engine has the concept of foreign keys at all. CH
   * is `false`. The distinction from `capabilities.foreignKeys` is
   * *engine class* vs *per-table coverage* — false here means the
   * FK section's "unsupported" message reads as a permanent
   * engine property, not a coverage gap.
   */
  hasForeignKeys: boolean;
  /**
   * Copy rendered in the structure view's Foreign Keys section
   * when `capabilities.foreignKeys` is false.
   */
  foreignKeysUnsupportedCopy: string;
  /**
   * Banner shown above the schema-relationship map when the
   * engine has no foreign keys but the load returned tables. Null
   * for engines where this situation doesn't arise.
   */
  schemaMapNoForeignKeysCopy: string | null;
  /**
   * Section labels — "Primary key" / "Sorting key", "Indexes" /
   * "Skip indices", etc.
   */
  labels: StructureLabels;
};

// ---------------------------------------------------------------------------
// KeyValue engine policy
// ---------------------------------------------------------------------------

/** Redis value types we render. Stream + JSON included; bitmap/HLL/geo
 * surface as their underlying type with a secondary panel. */
export type RedisKeyType =
  | "string"
  | "hash"
  | "list"
  | "set"
  | "zset"
  | "stream"
  | "json";

export type KeyValueEnginePolicy = {
  storageClass: "keyvalue";
  /** Default `dbNumber` on the connection form. Always 0 for v1. */
  defaultDbNumber: number;
  /** Maximum `dbNumber` accepted by the form. 15 on standalone Redis. */
  maxDbNumber: number;
  /** Whether pub/sub is supported at all. Cluster mode quirks later. */
  pubSubSupported: boolean;
  /** Whether MULTI/EXEC transactions are supported. Cluster: cross-slot fails. */
  transactionsSupported: boolean;
  /** Default separator used by the prefix-tree keyspace browser. */
  defaultSeparator: string;
  /**
   * Commands that require typed-confirmation before sending — kept
   * in sync with `src-tauri/src/redis/destructive_commands.rs` via
   * codegen. See ADR-0009. (Phase 1.1: hand-mirrored; codegen wires
   * in Phase 1.5 if drift becomes an issue.)
   */
  destructiveCommandsHard: readonly string[];
  destructiveCommandsSoft: readonly string[];
};

// ---------------------------------------------------------------------------
// Discriminated union
// ---------------------------------------------------------------------------

type EngineBase = {
  engine: DatabaseEngine;
  connectionForm: ConnectionFormPolicy;
};

export type RelationalPolicy = EngineBase & RelationalEnginePolicy;
export type KeyValuePolicy = EngineBase & KeyValueEnginePolicy;

export type EnginePolicy = RelationalPolicy | KeyValuePolicy;

const RELATIONAL_STRUCTURE_DEFAULTS: StructureLabels = {
  primaryKey: "Primary key",
  primaryKeyBadge: "PK",
  indexes: "Indexes",
  noPrimaryKey: "This table has no primary key.",
  noIndexes: "No indexes defined.",
};

const KEYVALUE_REDIS_DESTRUCTIVE_HARD = [
  "FLUSHDB",
  "FLUSHALL",
  "DEBUG",
  "SHUTDOWN",
  "CONFIG SET",
  "CONFIG RESETSTAT",
  "SCRIPT FLUSH",
  "SCRIPT KILL",
  "CLIENT KILL",
] as const;

const KEYVALUE_REDIS_DESTRUCTIVE_SOFT = ["KEYS"] as const;

const POLICIES: Record<DatabaseEngine, EnginePolicy> = {
  PostgreSQL: {
    engine: "PostgreSQL",
    storageClass: "relational",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: false,
      showRedisTls: false,
      showRedisDbNumber: false,
      defaultPort: 5432,
    },
    labels: { ...RELATIONAL_STRUCTURE_DEFAULTS },
  },
  MySQL: {
    engine: "MySQL",
    storageClass: "relational",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: false,
      showRedisTls: false,
      showRedisDbNumber: false,
      defaultPort: 3306,
    },
    labels: { ...RELATIONAL_STRUCTURE_DEFAULTS },
  },
  SQLite: {
    engine: "SQLite",
    storageClass: "relational",
    rowCountKind: "estimate",
    hasForeignKeys: true,
    foreignKeysUnsupportedCopy:
      "Foreign keys are not supported on this engine.",
    schemaMapNoForeignKeysCopy: null,
    connectionForm: {
      requiresHostAndAuth: false,
      showClickHouseHttp: false,
      showRedisTls: false,
      showRedisDbNumber: false,
      defaultPort: 0,
    },
    labels: { ...RELATIONAL_STRUCTURE_DEFAULTS },
  },
  ClickHouse: {
    engine: "ClickHouse",
    storageClass: "relational",
    rowCountKind: "exact",
    hasForeignKeys: false,
    foreignKeysUnsupportedCopy: "ClickHouse does not support foreign keys.",
    schemaMapNoForeignKeysCopy:
      "ClickHouse does not support foreign keys — showing tables only.",
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: true,
      showRedisTls: false,
      showRedisDbNumber: false,
      defaultPort: 8123,
    },
    labels: {
      primaryKey: "Sorting key",
      primaryKeyBadge: "ORDER BY",
      indexes: "Skip indices",
      noPrimaryKey: "This table has no sorting key.",
      noIndexes: "No skip indices defined.",
    },
  },
  Redis: {
    engine: "Redis",
    storageClass: "keyvalue",
    defaultDbNumber: 0,
    maxDbNumber: 15,
    pubSubSupported: true,
    transactionsSupported: true,
    defaultSeparator: ":",
    destructiveCommandsHard: KEYVALUE_REDIS_DESTRUCTIVE_HARD,
    destructiveCommandsSoft: KEYVALUE_REDIS_DESTRUCTIVE_SOFT,
    connectionForm: {
      requiresHostAndAuth: true,
      showClickHouseHttp: false,
      showRedisTls: true,
      showRedisDbNumber: true,
      defaultPort: 6379,
    },
  },
};

/**
 * Look up the UI policy for a given engine. Returns the policy
 * record verbatim; never null because `Record<DatabaseEngine, ...>`
 * forces every variant to have an entry.
 */
export function enginePolicy(engine: DatabaseEngine): EnginePolicy {
  return POLICIES[engine];
}

/**
 * Storage class for an engine. Cheap helper for components that
 * only care about the relational/keyvalue distinction without
 * narrowing on the full policy.
 */
export function storageClassFor(engine: DatabaseEngine): StorageClass {
  return POLICIES[engine].storageClass;
}

/**
 * Narrow an engine to its relational policy. Throws if called with
 * a keyvalue engine — this is a developer-error guard, used at the
 * entry points of relational-only components. The throw should be
 * unreachable at runtime because those components don't render for
 * Redis-class connections.
 */
export function relationalPolicy(engine: DatabaseEngine): RelationalPolicy {
  const policy = enginePolicy(engine);
  if (policy.storageClass !== "relational") {
    throw new Error(
      `relationalPolicy() called with keyvalue engine: ${engine}`,
    );
  }
  return policy;
}

/**
 * Narrow an engine to its keyvalue policy. Throws if called with a
 * relational engine — same developer-error-guard pattern as
 * `relationalPolicy()`.
 */
export function keyvaluePolicy(engine: DatabaseEngine): KeyValuePolicy {
  const policy = enginePolicy(engine);
  if (policy.storageClass !== "keyvalue") {
    throw new Error(
      `keyvaluePolicy() called with relational engine: ${engine}`,
    );
  }
  return policy;
}

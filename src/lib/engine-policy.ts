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

/**
 * Per-engine form-shape policy. A tagged union on `kind`; each
 * variant carries exactly the knobs its form needs. Multiple engines
 * may share a `kind` (PG and MySQL both render as `host-auth`); the
 * `kind` discriminator names the *form shape*, not the engine. The
 * engine still ships in the surrounding `EnginePolicy`.
 *
 * See ADR-0012 for the union shape and the rationale for grouping
 * PG/MySQL under one form kind while keeping them as separate
 * Connection-record variants (ADR-0011).
 */
export type ConnectionFormPolicy =
  | {
      kind: "host-auth";
      /** Default port placeholder when the user hasn't typed one. */
      defaultPort: number;
      /** Whether to render the SSL toggle (PG/MySQL only). */
      showSslToggle: boolean;
    }
  | {
      kind: "clickhouse-http";
      /** Port used when `useHttps` is off. */
      defaultPortHttp: number;
      /** Port used when `useHttps` is on. */
      defaultPortHttps: number;
    }
  | {
      kind: "redis";
      /** Default port placeholder when the user hasn't typed one. */
      defaultPort: number;
      /** Default DB number when the user hasn't typed one. */
      defaultDbNumber: number;
      /** Maximum DB number accepted on the form (15 on standalone). */
      maxDbNumber: number;
    }
  | {
      kind: "file";
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
      kind: "host-auth",
      defaultPort: 5432,
      showSslToggle: true,
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
      kind: "host-auth",
      defaultPort: 3306,
      showSslToggle: true,
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
    connectionForm: { kind: "file" },
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
      kind: "clickhouse-http",
      defaultPortHttp: 8123,
      defaultPortHttps: 8443,
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
      kind: "redis",
      defaultPort: 6379,
      defaultDbNumber: 0,
      maxDbNumber: 15,
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

/**
 * Shortcut to the connection-form policy for an engine. Forms read
 * this once at the engine-picker level and switch on `policy.kind` to
 * decide which fields to render.
 */
export function connectionFormPolicy(
  engine: DatabaseEngine,
): ConnectionFormPolicy {
  return POLICIES[engine].connectionForm;
}

// ---------------------------------------------------------------------------
// Connection-form validation
// ---------------------------------------------------------------------------

/**
 * Shared form-values shape — the union of every field any engine's
 * form might surface. Each variant of `ConnectionFormPolicy` knows
 * which fields apply to its kind; the validator below ignores
 * fields that don't.
 *
 * Slice 4 (#16) lifts this into the unified `ConnectionForm`
 * component; Slice 3 wires it through both existing forms.
 */
export type ConnectionFormValues = {
  name?: string;
  engine: DatabaseEngine;
  host?: string;
  database?: string;
  port?: number;
  user?: string;
  password?: string;
  role?: string;
  ssl?: boolean;
  useHttps?: boolean;
  urlPath?: string;
  dbNumber?: number;
  useTls?: boolean;
  verifyTlsCert?: boolean;
};

export type ConnectionFormIssue = {
  path: keyof ConnectionFormValues;
  message: string;
};

export type ConnectionFormMode = "new" | "edit";

/**
 * Validate connection-form values against the engine's form-policy
 * shape. `mode: "edit"` relaxes the password requirement for kinds
 * that otherwise require it (PG/MySQL/ClickHouse); the existing
 * backend rule "empty password = keep existing credential"
 * (`save_connection` in `lib.rs`) handles the substitution.
 *
 * Returns a flat list of issues; an empty list means the values are
 * valid. Each issue carries the form-field `path` it belongs to so
 * the consumer can surface inline errors next to the right input.
 *
 * Pure function — testable per `kind × mode` without spinning up a
 * form. ADR-0012 covers the broader connection-form unification.
 */
export function validateConnection(
  policy: ConnectionFormPolicy,
  value: ConnectionFormValues,
  mode: ConnectionFormMode,
): ConnectionFormIssue[] {
  const issues: ConnectionFormIssue[] = [];
  if (!value.name?.trim()) {
    issues.push({ path: "name", message: "Connection name is required" });
  }
  switch (policy.kind) {
    case "file": {
      if (!value.database?.trim()) {
        issues.push({ path: "database", message: "Database file is required" });
      }
      break;
    }
    case "host-auth": {
      validateHostFields(value, issues);
      validateDatabaseRequired(value, issues);
      validateUserRequired(value, issues);
      validatePasswordRequired(value, mode, issues);
      break;
    }
    case "clickhouse-http": {
      validateHostFields(value, issues);
      validateDatabaseRequired(value, issues);
      validateUserRequired(value, issues);
      validatePasswordRequired(value, mode, issues);
      break;
    }
    case "redis": {
      validateHostFields(value, issues);
      // Redis user + password are both optional (no-auth, password-only
      // for Redis ≤5 compat, or full ACL user+password on Redis 6+).
      // Database name doesn't apply — Redis uses dbNumber instead.
      const dbNumber = value.dbNumber ?? policy.defaultDbNumber;
      if (dbNumber < 0 || dbNumber > policy.maxDbNumber) {
        issues.push({
          path: "dbNumber",
          message: `DB number must be 0–${policy.maxDbNumber}`,
        });
      }
      break;
    }
  }
  return issues;
}

function validateHostFields(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  if (!value.host?.trim()) {
    issues.push({ path: "host", message: "Host is required" });
  }
  if (!value.port || value.port < 1 || value.port > 65535) {
    issues.push({
      path: "port",
      message: "Port must be between 1 and 65535",
    });
  }
}

function validateDatabaseRequired(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  if (!value.database?.trim()) {
    issues.push({ path: "database", message: "Database is required" });
  }
}

function validateUserRequired(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  if (!value.user?.trim()) {
    issues.push({ path: "user", message: "User is required" });
  }
}

function validatePasswordRequired(
  value: ConnectionFormValues,
  mode: ConnectionFormMode,
  issues: ConnectionFormIssue[],
): void {
  // `mode: "edit"` treats a blank password as "keep existing
  // credential" — the backend's `save_connection` only upserts the
  // credential when the password is non-empty (ADR-0010 §1).
  if (mode === "edit") return;
  if (!value.password?.trim()) {
    issues.push({ path: "password", message: "Password is required" });
  }
}

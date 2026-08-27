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

import {
  DESTRUCTIVE_HARD as KEYVALUE_REDIS_DESTRUCTIVE_HARD,
  DESTRUCTIVE_SOFT as KEYVALUE_REDIS_DESTRUCTIVE_SOFT,
} from "@/lib/redis/destructive-commands";
import type { DatabaseEngine, StorageClass } from "@/lib/store";
import type { PgTlsMode } from "@/lib/store/types";

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
      /**
       * Which TLS control the form renders. `"postgres-modes"` is the
       * libpq `sslmode` select with certificate paths (ADR-0025);
       * `"toggle"` is the single on/off switch MySQL keeps until it has
       * a verification story of its own.
       */
      tlsControls: "postgres-modes" | "toggle";
      /**
       * Whether the Advanced section renders the driver/session knobs
       * (statement timeout, search_path, default role — ADR-0013).
       * PG-only: the backend applies them through PG's `SET` grammar
       * and only `PgStoredConnection` carries the blob. MySQL shares
       * this form `kind` but not the field, so it reads `false` until
       * it gets its own ADR.
       */
      showDriverOptions: boolean;
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

const POLICIES = {
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
      tlsControls: "postgres-modes",
      showDriverOptions: true,
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
      tlsControls: "toggle",
      showDriverOptions: false,
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
} satisfies Record<DatabaseEngine, EnginePolicy>;

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
  environment?: import("@/lib/store").ConnectionEnvironment;
  safeMode?: import("@/lib/store").SafeMode;
  readOnly?: boolean;
  // Organization metadata (Plan 009, PAR-005). Engine-independent.
  folder?: string;
  isFavorite?: boolean;
  color?: import("@/lib/connection-colors").ConnectionColor;
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
  sshTunnelEnabled?: boolean;
  sshTunnelBastionServerId?: string;
  sshTunnelLocalBindHost?: string;
  sshTunnelLocalPort?: number;
  sshTunnelCompression?: boolean;
  sshTunnelKeepaliveIntervalSeconds?: number;
  sshTunnelKeepaliveWantReply?: boolean;
  sshTunnelJumpChain?: string[];
  sshTunnelProxyCommand?: string;
  // Driver/session knobs (ADR-0013). Only the `host-auth` policy with
  // `showDriverOptions` renders these; every other kind ignores them.
  // `defaultSearchPath` is the raw comma-separated text the user types
  // — `parseSearchPath` in `form-utils` splits it into the schema list
  // the backend stores.
  statementTimeoutMs?: number;
  idleInTransactionTimeoutMs?: number;
  connectTimeoutMs?: number;
  keepaliveSeconds?: number;
  defaultSearchPath?: string;
  defaultRole?: string;
  // PostgreSQL TLS (ADR-0025). Only the `postgres-modes` TLS control
  // renders these; `ssl` is derived from `tlsMode` when it is set.
  tlsMode?: PgTlsMode;
  tlsRootCertPath?: string;
  tlsClientCertPath?: string;
  tlsClientKeyPath?: string;
  tlsServerName?: string;
};

/**
 * Upper bound for the millisecond timeout knobs — 24h. Anything above
 * this is more likely a unit mix-up (seconds typed as ms) than intent.
 */
const MAX_TIMEOUT_MS = 86_400_000;

/** Upper bound for connect timeout — 10 minutes. */
const MAX_CONNECT_TIMEOUT_MS = 600_000;

/** Upper bound for keepalive idle — 2 hours; longer defeats the purpose. */
const MAX_KEEPALIVE_SECONDS = 7_200;

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
      if (policy.showDriverOptions) {
        validateDriverOptions(value, issues);
      }
      if (policy.tlsControls === "postgres-modes") {
        validateTlsFields(value, issues);
      }
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
  if (policy.kind !== "file") {
    validateTunnelFields(value, issues);
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

/**
 * Validate the ADR-0013 driver knobs. Every field is optional — a
 * blank input means "use the server default" and produces no issue.
 * The bounds exist to catch unit mix-ups (a user typing `30` meaning
 * seconds into a millisecond field is legal; typing `30000000` into
 * `connectTimeoutMs` almost certainly is not).
 */
function validateDriverOptions(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  // PG reads 0 as "no limit" for both session timeouts, so 0 is a
  // meaningful value here rather than an empty one.
  validateMsRange(
    value.statementTimeoutMs,
    "statementTimeoutMs",
    "Statement timeout",
    MAX_TIMEOUT_MS,
    issues,
  );
  validateMsRange(
    value.idleInTransactionTimeoutMs,
    "idleInTransactionTimeoutMs",
    "Idle-in-transaction timeout",
    MAX_TIMEOUT_MS,
    issues,
  );
  if (value.connectTimeoutMs !== undefined) {
    if (
      !Number.isInteger(value.connectTimeoutMs) ||
      value.connectTimeoutMs < 1 ||
      value.connectTimeoutMs > MAX_CONNECT_TIMEOUT_MS
    ) {
      issues.push({
        path: "connectTimeoutMs",
        message: `Connect timeout must be between 1 and ${MAX_CONNECT_TIMEOUT_MS} ms`,
      });
    }
  }
  validateKeepalive(value.keepaliveSeconds, issues);
  validateSearchPath(value.defaultSearchPath, issues);
}

function validateKeepalive(
  raw: number | undefined,
  issues: ConnectionFormIssue[],
): void {
  if (raw === undefined) return;
  if (!Number.isInteger(raw) || raw < 1 || raw > MAX_KEEPALIVE_SECONDS) {
    issues.push({
      path: "keepaliveSeconds",
      message: `Keepalive idle must be between 1 and ${MAX_KEEPALIVE_SECONDS} seconds`,
    });
  }
}

/**
 * Client-certificate authentication needs both halves. The backend
 * would refuse a lone path as `invalidLocalMaterial`, but only at
 * connect time — the form catches it first.
 */
function validateTlsFields(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  if (value.tlsMode === "disable") return;
  const cert = value.tlsClientCertPath?.trim();
  const key = value.tlsClientKeyPath?.trim();
  if (cert && !key) {
    issues.push({
      path: "tlsClientKeyPath",
      message: "Client key path is required with a client certificate",
    });
  }
  if (key && !cert) {
    issues.push({
      path: "tlsClientCertPath",
      message: "Client certificate path is required with a client key",
    });
  }
}

function validateMsRange(
  raw: number | undefined,
  path: keyof ConnectionFormValues,
  label: string,
  max: number,
  issues: ConnectionFormIssue[],
): void {
  if (raw === undefined) return;
  if (!Number.isInteger(raw) || raw < 0 || raw > max) {
    issues.push({
      path,
      message: `${label} must be between 0 and ${max} ms`,
    });
  }
}

/**
 * The search path is typed as free text and split on commas. The
 * backend double-quotes each entry before it reaches `SET search_path`
 * (`quote_double` in `postgres/pool.rs`), so injection isn't the
 * concern — a stray `"` inside an entry is, because it would land in
 * a quoted identifier the user didn't mean to write.
 */
function validateSearchPath(
  raw: string | undefined,
  issues: ConnectionFormIssue[],
): void {
  if (raw === undefined || !raw.trim()) return;
  const entries = raw.split(",").map((entry) => entry.trim());
  if (entries.some((entry) => !entry)) {
    issues.push({
      path: "defaultSearchPath",
      message: "Search path entries cannot be empty",
    });
  }
  if (entries.some((entry) => entry.includes('"'))) {
    issues.push({
      path: "defaultSearchPath",
      message: 'Search path entries cannot contain a double quote (")',
    });
  }
}

function validateTunnelFields(
  value: ConnectionFormValues,
  issues: ConnectionFormIssue[],
): void {
  if (!value.sshTunnelEnabled) {
    return;
  }
  if (!value.sshTunnelBastionServerId?.trim()) {
    issues.push({
      path: "sshTunnelBastionServerId",
      message: "Bastion Server is required",
    });
  }
  if (
    value.sshTunnelLocalPort !== undefined &&
    (value.sshTunnelLocalPort < 1 || value.sshTunnelLocalPort > 65535)
  ) {
    issues.push({
      path: "sshTunnelLocalPort",
      message: "Local port must be between 1 and 65535",
    });
  }
  if (
    value.sshTunnelKeepaliveIntervalSeconds !== undefined &&
    (value.sshTunnelKeepaliveIntervalSeconds < 2 ||
      value.sshTunnelKeepaliveIntervalSeconds > 3600)
  ) {
    issues.push({
      path: "sshTunnelKeepaliveIntervalSeconds",
      message: "Keepalive interval must be between 2 and 3600 seconds",
    });
  }
  const jumpChain = (value.sshTunnelJumpChain ?? [])
    .map((bastionId) => bastionId.trim())
    .filter(Boolean);
  if (jumpChain.includes(value.sshTunnelBastionServerId?.trim() ?? "")) {
    issues.push({
      path: "sshTunnelJumpChain",
      message: "Jump chain cannot include the selected Bastion Server",
    });
  }
  if (new Set(jumpChain).size !== jumpChain.length) {
    issues.push({
      path: "sshTunnelJumpChain",
      message: "Jump chain cannot include duplicate Bastion Servers",
    });
  }
}

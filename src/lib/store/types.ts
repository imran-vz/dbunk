/**
 * Shared DTOs for the workspace store.
 *
 * Slice files import their types from here so they don't need to
 * import from each other (which the slice-isolation discipline
 * forbids — see `store/README.md`). External consumers continue to
 * import types from `@/lib/store` (the directory barrel re-exports).
 *
 * Keep this file types-only. Helpers / pure functions live with the
 * slice that uses them. Cross-slice helpers (`formatLatencyMs`,
 * `errorToMessage`) live in `@/lib/format` and `@/lib/tauri`.
 */

import type { ColumnChangeKind } from "@/lib/ddl/postgres";
import type {
  SchemaMapAttrMode,
  SchemaMapPosition,
  SchemaMapPrefs,
  SchemaMapRouting,
  SchemaRelationships,
} from "@/lib/schema-graph";
import type {
  BrowseCursor,
  BrowseExactCountResult,
  BrowseFilter,
  BrowseSortKey,
  BrowseTableResult,
  TableBrowseError,
  TableBrowseFilterMode,
  TableGridPrefs,
} from "@/lib/table-browse";
import type { ThemeMode, ThemePreset } from "@/lib/theme";

// ---------------------------------------------------------------------------
// Engine + storage class
// ---------------------------------------------------------------------------

export type DatabaseEngine =
  | "PostgreSQL"
  | "MySQL"
  | "ClickHouse"
  | "SQLite"
  | "Redis";

/**
 * Top-level engine class. Relational engines share schemas/tables/
 * rows/SQL; keyvalue engines share a keyspace of typed keys. Derived
 * from `DatabaseEngine`; see ADR-0008 and `engine-policy.ts`.
 */
export type StorageClass = "relational" | "keyvalue";

// ---------------------------------------------------------------------------
// Redis-specific identity
// ---------------------------------------------------------------------------

export type RedisModuleInfo = {
  name: string;
  version: string;
};

/**
 * Connect-time pipeline result for Redis — surfaced in the
 * post-test-connection banner on the new-connection form. Every
 * field is optional because managed Redis (Upstash hobby tier,
 * locked-down ACLs) often restricts `INFO` sections or
 * `MODULE LIST` and we degrade per-field rather than failing.
 */
export type RedisCapabilities = {
  serverVersion?: string;
  /** `master` or `replica`. Drives auto-read-only (ADR-0009). */
  role?: string;
  connectedSlaves?: number;
  modules?: RedisModuleInfo[];
  dbSize?: number;
  maxmemoryPolicy?: string;
};

// ---------------------------------------------------------------------------
// Credentials + app settings
// ---------------------------------------------------------------------------

export type CredentialStorageMode =
  | "keychain"
  | "encrypted-sqlite"
  | "plain-sqlite";

export type CredentialState = "needs-onboarding" | "needs-unlock" | "ready";

/**
 * `theme` and `themePreset` are canonical persistence for the user's
 * theme choice; `localStorage["dbunk.theme"]` and
 * `localStorage["dbunk.theme.preset"]` are boot-cache mirrors so a
 * pre-paint script can apply the resolved theme before React
 * hydrates. The Rust side omits a field when no choice is yet
 * stored, which the TS layer treats as `"system"` / `"default"`
 * respectively.
 */
export type AppSettingsSnapshot = {
  onboardingCompleted: boolean;
  credentialStorageMode: CredentialStorageMode | null;
  credentialState: CredentialState;
  configDir: string;
  theme?: ThemeMode;
  themePreset?: ThemePreset;
};

export type AppSettingsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Bastion servers + SSH tunnels
// ---------------------------------------------------------------------------

export type BastionAuthMethod =
  | "password"
  | "privateKeyPath"
  | "privateKeyContent";

export type BastionServer = {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  authMethod: BastionAuthMethod;
  privateKeyPath?: string;
  hostKeyFingerprint?: string;
  createdAt: string;
  updatedAt: string;
  hasPassword: boolean;
  hasPrivateKeyContent: boolean;
  hasPassphrase: boolean;
};

export type SecretChange =
  | { action: "keep" }
  | { action: "set"; value: string }
  | { action: "clear" };

export type SaveBastionServerInput = {
  id: string;
  name: string;
  host: string;
  port: number;
  user: string;
  authMethod: BastionAuthMethod;
  privateKeyPath?: string;
  password: SecretChange;
  privateKeyContent: SecretChange;
  passphrase: SecretChange;
};

export type BastionStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Managed Servers (ADR-0019)
// ---------------------------------------------------------------------------

/** A Docker-provisioned local database dbunk owns the lifecycle of. */
export type ManagedServer = {
  id: string;
  name: string;
  engine: DatabaseEngine;
  /** Major-version image tag, e.g. `"17"` for `postgres:17`. */
  version: string;
  port: number;
  containerName: string;
  volumeName: string;
  database: string;
  user: string;
  /** The auto-created Connection pointing at this server. */
  connectionId: string | null;
  createdAt: string;
};

/** Status is derived live from Docker, never trusted from storage. */
export type ManagedServerStatus =
  | "running"
  | "starting"
  | "stopped"
  | "orphaned";

export type ManagedServerWithStatus = ManagedServer & {
  status: ManagedServerStatus;
  /** For `orphaned`: whether Recreate can restore data. */
  volumeExists: boolean;
};

export type DockerStatus = {
  available: boolean;
  version: string | null;
  error: string | null;
};

export type ProvisionManagedServerInput = {
  name: string;
  engine: "PostgreSQL" | "MySQL";
  version: string;
  port?: number;
  database?: string;
  user?: string;
};

export type ProvisionManagedServerResult = {
  server: ManagedServer;
  connectionId: string;
  /** Shown once post-create for copy into the project's env. */
  connectionString: string;
};

export type ManagedServersStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "error"; error: string };

export type SshTunnelConfig = {
  enabled: boolean;
  bastionServerId?: string;
  localBindHost?: string;
  localPort?: number;
  compression?: boolean;
  keepaliveIntervalSeconds?: number;
  keepaliveWantReply?: boolean;
  jumpChain?: string[];
  proxyCommand?: string;
};

// ---------------------------------------------------------------------------
// Connection records
// ---------------------------------------------------------------------------

/**
 * Connection records are per-engine tagged unions discriminated by
 * `engine`. The Rust backend serializes the same shape via internally-
 * tagged enums (ADR-0010); TypeScript narrowing makes engine-specific
 * fields unreachable on the wrong variant.
 *
 * See ADR-0011 for the rationale (strict per-engine, sits below the
 * storage-class fork from ADR-0008).
 */
type ConnectionCommon = {
  id: string;
  name: string;
  database: string;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  /** Operator-selected deployment identity. Missing legacy values resolve to development. */
  environment?: ConnectionEnvironment;
  /** Per-connection policy override. Missing legacy values resolve to inherit. */
  safeMode?: SafeMode;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
  /** Single-level organization group; empty/missing = ungrouped. Plan 009. */
  folder?: string;
  isFavorite?: boolean;
  /** User-picked presentation color token; see `connection-colors.ts`. */
  color?: import("@/lib/connection-colors").ConnectionColor;
};

export type ConnectionEnvironment =
  | "development"
  | "test"
  | "staging"
  | "production";

export type SafeMode = "inherit" | "disabled" | "protected" | "strict";

/** libpq `sslmode` vocabulary; persisted and sent in this spelling (ADR-0025). */
export const PG_TLS_MODES = [
  "disable",
  "prefer",
  "require",
  "verify-ca",
  "verify-full",
] as const;
export type PgTlsMode = (typeof PG_TLS_MODES)[number];

/**
 * PostgreSQL TLS mode and certificate *paths* (never contents), persisted
 * as one JSON blob on the connection record (migration 18, ADR-0025).
 * Absent on legacy records, which resolve through `ssl`.
 */
export type PgTlsOptions = {
  mode: PgTlsMode;
  rootCertPath?: string;
  clientCertPath?: string;
  clientKeyPath?: string;
  /** Host name the certificate must match when it differs from `host`. */
  serverName?: string;
};

/** Why a TLS-protected connect failed (ADR-0025). */
export const TLS_FAILURE_KINDS = [
  "serverRefusedTls",
  "certificateUntrusted",
  "hostnameMismatch",
  "clientCertificateRejected",
  "invalidLocalMaterial",
  "handshakeFailed",
] as const;
export type TlsFailureKind = (typeof TLS_FAILURE_KINDS)[number];
/** Narrow a decoded wire string; unknown values are not a TLS failure kind. */
export function isTlsFailureKind(
  value: string | undefined,
): value is TlsFailureKind {
  return TLS_FAILURE_KINDS.some((kind) => kind === value);
}

// ---------------------------------------------------------------------------
// Staged connection diagnosis (ADR-0025) — wire mirrors of
// `src-tauri/src/diagnosis.rs`. Rendered by Plan 012.
// ---------------------------------------------------------------------------

export type DiagnosisStageKind =
  | "tunnel"
  | "dns"
  | "tcp"
  | "tls"
  | "authentication"
  | "database";

export type DiagnosisFailureKind =
  | TlsFailureKind
  | "tunnelFailed"
  | "dnsUnresolvable"
  | "connectionRefused"
  | "timedOut"
  | "unreachable"
  | "authenticationFailed"
  | "databaseMissing"
  | "other";

export type DiagnosisSkipReason =
  | "noTunnel"
  | "tlsDisabled"
  | "blockedByEarlierFailure"
  | "notApplicable";

export type PoolHostnameVerification = "full" | "caOnly" | "notApplicable";

export type DiagnosisStageDetail =
  | { kind: "tunnel"; localEndpoint: string }
  | { kind: "dns"; addresses: string[] }
  | {
      kind: "tls";
      /** From `pg_stat_ssl` when reachable — the only honest source for `prefer`. */
      encrypted: boolean;
      protocol: string | null;
      cipher: string | null;
      certificateVerified: boolean;
      hostnameVerified: boolean;
      clientCertificatePresented: boolean;
      poolHostnameVerification: PoolHostnameVerification;
    }
  | { kind: "database"; serverVersion: string };

export type DiagnosisStageResult =
  | { status: "passed"; elapsedMs: number; detail?: DiagnosisStageDetail }
  | {
      status: "failed";
      elapsedMs: number;
      kind: DiagnosisFailureKind;
      message: string;
    }
  | { status: "skipped"; reason: DiagnosisSkipReason };

export type DiagnosisStage = {
  stage: DiagnosisStageKind;
  result: DiagnosisStageResult;
};

export type DiagnosisOutcome =
  | { kind: "reachable"; latencyMs: number }
  | { kind: "failed"; stage: DiagnosisStageKind };

export type DiagnosisWarning =
  | "notEncrypted"
  | "poolHostnameVerificationCaOnly"
  | "productionWithoutVerification";

export type ConnectionDiagnosis = {
  engine: DatabaseEngine;
  /** Fixed order: tunnel, dns, tcp, tls, authentication, database. */
  stages: DiagnosisStage[];
  outcome: DiagnosisOutcome;
  warnings: DiagnosisWarning[];
};

/**
 * Per-connection driver/session knobs, persisted as a JSON blob on
 * the Postgres connection record. See ADR-0013. Each field is
 * optional — `undefined` means "use the server default".
 *
 * `connectTimeoutMs` bounds the initial handshake on every driver.
 * `keepaliveSeconds` is applied on the dedicated driver (query sessions,
 * table browse, result mutation); the pooled metadata driver (SQLx) has
 * no keepalive option, so metadata and admin queries keep the OS default
 * (ADR-0025).
 */
export type PgDriverOptions = {
  statementTimeoutMs?: number;
  idleInTransactionTimeoutMs?: number;
  connectTimeoutMs?: number;
  keepaliveSeconds?: number;
  defaultSearchPath?: string[];
  defaultRole?: string;
};

export type PgStoredConnection = ConnectionCommon & {
  engine: "PostgreSQL";
  readOnly?: boolean;
  /** Legacy TLS on/off mirror; `tlsOptions.mode` is authoritative when
   *  present and the backend keeps this equal to `mode !== "disable"`.
   *  Distinct from ClickHouse `useHttps` and Redis `useTls`. */
  ssl: boolean;
  /** TLS verification mode and certificate paths (ADR-0025). */
  tlsOptions?: PgTlsOptions;
  /** Optional driver/session knobs applied after every connect.
   *  See ADR-0013. Missing or empty fields fall back to PG defaults. */
  driverOptions?: PgDriverOptions;
  sshTunnel?: SshTunnelConfig;
};
export type MySqlStoredConnection = ConnectionCommon & {
  engine: "MySQL";
  readOnly?: boolean;
  ssl: boolean;
  sshTunnel?: SshTunnelConfig;
};
export type SqliteStoredConnection = ConnectionCommon & {
  engine: "SQLite";
  readOnly?: boolean;
};
export type ClickHouseStoredConnection = ConnectionCommon & {
  engine: "ClickHouse";
  readOnly?: boolean;
  /** Connect over HTTPS instead of HTTP. */
  useHttps: boolean;
  /** URL path prefix for proxied deployments (e.g. `/clickhouse`). */
  urlPath: string;
  sshTunnel?: SshTunnelConfig;
};
export type RedisStoredConnection = ConnectionCommon & {
  engine: "Redis";
  /** Which numbered DB (0–15 on standalone). */
  dbNumber: number;
  /** Connect over TLS (`rediss://`). */
  useTls: boolean;
  /** Verify the TLS certificate when useTls is on. */
  verifyTlsCert: boolean;
  /** Belt-and-braces safety toggle. When true, the backend refuses
   *  every write for this connection, independent of the replica-role
   *  check. See ADR-0009. */
  readOnly: boolean;
  sshTunnel?: SshTunnelConfig;
};

export type StoredConnection =
  | PgStoredConnection
  | MySqlStoredConnection
  | SqliteStoredConnection
  | ClickHouseStoredConnection
  | RedisStoredConnection;

type ConnectionRuntimeFields = {
  status: "Connected" | "Read only" | "Disconnected";
  latency: string;
  errorMessage?: string;
};

/**
 * The one definition of "this connection is live". Every surface that
 * gates on status (palette index, table session, workbench, header,
 * health checks) reads this so a new status literal is handled once.
 */
export const isConnectedStatus = (
  status: ConnectionRuntimeFields["status"] | undefined,
): boolean => status === "Connected" || status === "Read only";

export type PgConnection = PgStoredConnection & ConnectionRuntimeFields;
export type MySqlConnection = MySqlStoredConnection & ConnectionRuntimeFields;
export type SqliteConnection = SqliteStoredConnection & ConnectionRuntimeFields;
export type ClickHouseConnection = ClickHouseStoredConnection &
  ConnectionRuntimeFields;
export type RedisConnection = RedisStoredConnection & ConnectionRuntimeFields;

export type Connection =
  | PgConnection
  | MySqlConnection
  | SqliteConnection
  | ClickHouseConnection
  | RedisConnection;

// ---------------------------------------------------------------------------
// Per-tab status shapes
// ---------------------------------------------------------------------------

/**
 * Lifecycle state of an in-flight `runQuery` invocation — the
 * **best-effort view** the store keeps so the Run button can stay
 * disabled and the panel can render "Running…" across tab unmounts.
 * Terminal state is NOT carried here; callers get it from the awaited
 * `QueryOutcome` returned by `runQuery`. Absence from `queryStatus`
 * means "nothing in flight." See CONTEXT.md — Query Outcome.
 */
export type QueryStatus = { state: "running" | "cancelling" };

export type QueryExecutionStatus =
  | "running"
  | "completed"
  | "failed"
  | "cancelled"
  | "lost";
export type QueryExecutionTerminalStatus = Exclude<
  QueryExecutionStatus,
  "running" | "lost"
>;

export type QueryTransactionMode = "autocommit" | "manual";
export type QueryTransactionStatus = "idle" | "active" | "failed" | "unknown";
export type QueryTransactionIsolation =
  | "readCommitted"
  | "repeatableRead"
  | "serializable";
export type QueryTransactionSnapshot = {
  mode: QueryTransactionMode;
  status: QueryTransactionStatus;
  manualIsolation: QueryTransactionIsolation;
};
export type QueryDatabaseError = {
  code: string | null;
  message: string;
  severity: string | null;
  position: number | null;
};
export type StatementClassSummary =
  import("@/lib/safety-policy").StatementClassSummary;
export type QuerySessionError =
  | { kind: "unsupportedEngine" }
  | { kind: "connectionClosing" }
  | { kind: "sessionLimitReached"; limit: string }
  | { kind: "sessionNotFound" }
  | { kind: "ownerMismatch" }
  | { kind: "executionInProgress" }
  | { kind: "invalidSequence" }
  | {
      kind: "invalidTransactionTransition";
      status: QueryTransactionStatus;
      attemptedAction: string;
      allowedActions: string[];
    }
  | { kind: "transactionStateUnknown"; canRecheck: boolean }
  | { kind: "transactionObserverUnavailable" }
  | { kind: "connectionLost" }
  | { kind: "tlsFailed"; tlsKind: TlsFailureKind; message: string }
  | { kind: "policyBlocked"; reason: string }
  | { kind: "policyNeedsConfirmation"; statements: StatementClassSummary[] }
  | { kind: "timeout"; operation: string }
  | ({ kind: "database" } & QueryDatabaseError);
export type QueryNotice = { severity: string; message: string };
export type QueryResultSet = {
  index: number;
  columns: Array<string | null>;
  /** Row batches as received. Flatten at render — never copy prior rows on append. */
  rowChunks: Array<Array<Array<string | null>>>;
  rowCount: number;
  partial: boolean;
  completed: boolean;
};
export type QueryExecutionTombstone = {
  status: QueryExecutionStatus;
  resultCount: number;
  rowCount: number;
  noticeCount: number;
  omittedCount: number;
  runtimeMs: number;
  releasedBytes: number;
  completedAt: string;
  reason: "globalBudget";
};
export type QueryExecution = {
  id: string;
  status: QueryExecutionStatus;
  startedAt: string;
  completedAt: string | null;
  runtimeMs: number;
  resultSets: QueryResultSet[];
  notices: QueryNotice[];
  error: QueryDatabaseError | null;
  omittedRows: number;
  omittedResultSets: number;
  omittedNotices: number;
  omittedMetadataBytes: number;
  truncationReasons: string[];
  retainedBytes: number;
  tombstone: QueryExecutionTombstone | null;
};
export type QuerySessionState = {
  id: string;
  tabId: string;
  connectionId: string;
  generation: number | null;
  transaction: QueryTransactionSnapshot;
  execution: QueryExecution | null;
  lastViewedAt: number;
  budgetOwners: Array<{ tabId: string; label: string; retainedBytes: number }>;
  state: "opening" | "open" | "lost" | "closed";
  policyRefusal?: string | null;
};

/**
 * Terminal result of one `runQuery` invocation — the caller-facing
 * outcome of executing one SQL statement. A tagged union on `kind`:
 * `"completed" | "failed" | "noop"`, returned by the action so the
 * caller can await its own operation's result. `noop` covers the
 * short-circuit cases (no tab, wrong tab kind, already running,
 * empty query, no backend); the UI does not render a banner for
 * it. See CONTEXT.md — Query Outcome.
 */
export type QueryOutcome =
  | {
      kind: "completed";
      runtimeMs: number;
      rowCount: number;
      /**
       * Legacy `run_query` path stores rows here (and in `queryPreviews`).
       * Persistent session completions leave this null — the Results tab
       * reads `querySessions[tabId].execution` as the single retained copy.
       */
      preview: QueryPreviewData | null;
    }
  | { kind: "failed"; reason: string }
  | { kind: "cancelled" }
  | { kind: "noop" };

export type TableLoadStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

export type TableBrowseLoadStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: TableBrowseError };

export type TableBrowseTabState = {
  tabId: string;
  connectionId: string;
  schema: string;
  table: string;
  generation: number;
  typedFilters: BrowseFilter[];
  rawFilterText: string;
  filterMode: TableBrowseFilterMode;
  sort: BrowseSortKey[];
  pageSize: number;
  page: number;
  cursorStack: Array<BrowseCursor | null>;
  /** Monotonic within this tab generation; never derived from the in-flight slot. */
  nextRequestToken: number;
  inflightRequestId: number | null;
  appliedRequestId: number | null;
  result: BrowseTableResult | null;
  loadStatus: TableBrowseLoadStatus;
  countStatus: TableBrowseLoadStatus;
  exactCount: BrowseExactCountResult | null;
  prefsLoaded: boolean;
  prefs: TableGridPrefs;
};

export type LoadingStatus = TableLoadStatus;

// ---------------------------------------------------------------------------
// Table data + structure
// ---------------------------------------------------------------------------

export type TableRef = {
  connectionId: string;
  schema: string;
  table: string;
};

export type TableDataState = {
  connectionId: string;
  schema: string;
  table: string;
  columns: string[];
  rows: string[][];
  page: number;
  pageSize: number;
  totalRows?: number;
  runtimeMs: number;
};

export const tableDataKey = (
  connectionId: string,
  schema: string,
  table: string,
) => `${connectionId}::${schema}::${table}`;

export const tableStructureKey = (
  connectionId: string,
  schema: string,
  table: string,
) => `${connectionId}::${schema}::${table}`;

export const tableSessionKey = (ref: TableRef) =>
  tableDataKey(ref.connectionId, ref.schema, ref.table);

export type ColumnInfo = {
  name: string;
  dataType: string;
  nullable: boolean;
  defaultValue: string | null;
  isPrimaryKey: boolean;
  ordinalPosition: number;
  /** ClickHouse-only: `"MATERIALIZED"` / `"ALIAS"` / `"EPHEMERAL"`
   *  for derived columns. PostgreSQL leaves this `null`. Drives the
   *  "derived" icon in the data-grid column header. */
  derivationKind?: string | null;
};

export type ForeignKeyInfo = {
  name: string;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: string | null;
  onDelete: string | null;
};

export type IndexInfo = {
  name: string;
  columns: string[];
  isUnique: boolean;
  isPrimary: boolean;
  method: string | null;
};

export type ConstraintInfo = {
  name: string;
  kind: string;
  definition: string;
};

export type TriggerEnabledState = "origin" | "disabled" | "replica" | "always";

export type PolicyCommand = "ALL" | "SELECT" | "INSERT" | "UPDATE" | "DELETE";

export type TriggerInfo = {
  name: string;
  /** `BEFORE` | `AFTER` | `INSTEAD OF`. */
  timing: string;
  /** Any of `INSERT`, `UPDATE`, `DELETE`, `TRUNCATE`. */
  events: string[];
  /** Columns of an `UPDATE OF` list; empty otherwise. */
  updateColumns: string[];
  /** `ROW` | `STATEMENT`. */
  level: string;
  enabled: TriggerEnabledState;
  functionSchema: string;
  functionName: string;
  /** `pg_get_triggerdef` output, verbatim. */
  definition: string;
};

export type PolicyInfo = {
  name: string;
  permissive: boolean;
  command: PolicyCommand;
  /** Role names as PostgreSQL reports them; `public` is the pseudo-role. */
  roles: string[];
  using: string | null;
  withCheck: string | null;
};

/** One explicit ACL entry; a relation with no explicit ACL reports none. */
export type PrivilegeInfo = {
  /** Role name, or `PUBLIC`. */
  grantee: string;
  privilege: string;
  grantable: boolean;
};

export type RowSecurityInfo = {
  enabled: boolean;
  forced: boolean;
};

export type StructureCapabilities = {
  columns: boolean;
  primaryKey: boolean;
  foreignKeys: boolean;
  indexes: boolean;
  constraints: boolean;
  /** Whether `triggers` / `policies` / `privileges` are populated (PostgreSQL). */
  triggers: boolean;
  policies: boolean;
  privileges: boolean;
  /** Whether new rows can be inserted via the row editor. */
  canInsertRows: boolean;
  /** Whether existing cells can be updated via the row editor. */
  canUpdateRows: boolean;
  /** Whether rows can be deleted via the row editor. */
  canDeleteRows: boolean;
  /** Whether ALTER TABLE-style schema edits are supported. */
  canAlterSchema: boolean;
  /**
   * "exact" — identity columns guarantee at most one matching row.
   * "best-effort" — identity may match multiple rows (ClickHouse).
   */
  uniquenessGuarantee: "exact" | "best-effort";
};

export type TableStructure = {
  columns: ColumnInfo[];
  primaryKey: string[] | null;
  foreignKeys: ForeignKeyInfo[];
  indexes: IndexInfo[];
  constraints: ConstraintInfo[];
  /** PostgreSQL table security and behaviour; empty on other engines. */
  triggers: TriggerInfo[];
  policies: PolicyInfo[];
  privileges: PrivilegeInfo[];
  rowSecurity: RowSecurityInfo | null;
  capabilities: StructureCapabilities;
  /** Engine-specific extension fields. Populated only for ClickHouse. */
  tableEngine?: string;
  partitionBy?: string;
  sampleBy?: string;
};

export type TableStructureStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

/**
 * One queued table-structure edit. A table's pending list is homogeneous:
 * legacy frontend-rendered column changes and PostgreSQL object operations
 * never share a batch.
 */
export type StructureChange =
  | { kind: "column"; change: ColumnChangeKind }
  | { kind: "pg-op"; op: PgObjectOp };

export type PendingChange = {
  id: string;
  schema: string;
  table: string;
  change: StructureChange;
};

/**
 * Lifecycle state of an in-flight DDL commit — the **best-effort view**
 * the store keeps so the Commit button can stay disabled and labelled
 * "Committing..." across tab unmounts. Terminal state is NOT carried
 * here; callers get it from the awaited `DDLOutcome` returned by
 * `commitStructureChanges`. Absence from `structureCommitStatus` means
 * "nothing in flight." See CONTEXT.md — DDL Outcome.
 */
export type StructureCommitStatus = { state: "running" };

/**
 * Terminal result of one `commitStructureChanges` attempt — the
 * caller-facing outcome of executing a batch of DDL Statements. Always
 * synchronous today (no async tracking, unlike Edit Outcome). The
 * `noop` variant is returned when there were no pending changes —
 * distinct from `completed` so a future caller without a UI guard
 * doesn't render a "Committed in 0 ms" banner for nothing. See
 * CONTEXT.md — DDL Outcome.
 */
export type DDLOutcome =
  | { kind: "completed"; runtimeMs: number }
  | { kind: "failed"; reason: string }
  | { kind: "noop" };

/**
 * Lifecycle state of an in-flight Edit (cell-edit commit, row insert,
 * row delete) — the **best-effort view** the store keeps so the UI
 * badge can render across tab unmounts. Terminal state is NOT carried
 * here; callers get it from the awaited `EditOutcome` returned by the
 * action. Absence from `tableEditsCommitStatus` means "nothing in
 * flight." See CONTEXT.md — Edit Outcome.
 */
export type TableEditsCommitStatus =
  | { state: "running" }
  | {
      state: "queued";
      database: string;
      table: string;
      mutationIds: string[];
      runtimeMs: number;
    };

/**
 * Terminal result of one Edit attempt, returned by the store action
 * that initiated the write. Synchronous engines (PG/MySQL/SQLite)
 * resolve to `completed` or `failed`; `timeout` is reachable only on
 * async engines (ClickHouse, via Pending Mutation tracking). The
 * `noop` variant is returned when the action found nothing to do
 * (no edits, no rows, no payload after filtering) — distinct from
 * `completed` so callers don't render a misleading "saved 0 rows"
 * banner for a click that did no work. See CONTEXT.md — Edit Outcome.
 */
export type EditOutcome =
  | { kind: "completed"; runtimeMs: number; rowsAffected?: number }
  | { kind: "failed"; reason: string; mutationId?: string }
  | { kind: "timeout"; remaining: string[] }
  | { kind: "noop" };

export type TableSessionCapabilities = {
  structureLoaded: boolean;
  isReadOnly: boolean;
  isWriting: boolean;
  canAddRow: boolean;
  canDeleteRows: boolean;
  canEditCells: boolean;
};

export type TableSessionSnapshot = {
  ref: TableRef;
  key: string;
  data?: TableDataState;
  structure?: TableStructure;
  loadStatus?: TableLoadStatus;
  structureStatus?: TableStructureStatus;
  writeStatus?: TableEditsCommitStatus;
  edits: Record<number, Record<number, string>>;
  capabilities: TableSessionCapabilities;
};

export type SchemaRelationshipsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

export type DatabaseOverviewStats = {
  databaseSizeBytes: number;
  tableSizeBytes: number;
  indexSizeBytes: number;
  tableCount: number;
  schemaCount: number;
  rowCountEstimate: number;
  indexCount: number;
  connectionCount: number;
};

export type DatabaseOverviewStatsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

/**
 * One row per user-visible relation (table, view, materialised view)
 * in a relational connection, populated by `loadRelationStats`. Drives
 * the Tables and Schemas sub-tabs. On non-PG engines the action
 * resolves to an empty list (the Schemas sub-tab is gated to PG; the
 * Tables sub-tab degrades to schema/name/kind columns sourced from
 * `schemaExplorer`).
 */
export type RelationInfo = {
  schema: string;
  name: string;
  /** "table" | "view" | "materialized view" — drives the kind badge. */
  kind: string;
  /** PG planner estimate (`pg_class.reltuples`). Zero for views. */
  rowCountEstimate: number;
  /** `pg_total_relation_size` bytes (table + TOAST + indexes). */
  totalSizeBytes: number;
};

export type RelationStatsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

/**
 * One row from `pg_settings` — a single GUC parameter. The Details
 * sub-tab groups by `category`, surfaces `shortDesc` as a tooltip,
 * and highlights any row where `source !== "default"` to flag
 * operator overrides.
 */
export type PgSetting = {
  name: string;
  setting: string;
  unit: string | null;
  category: string;
  shortDesc: string | null;
  /**
   * Where the setting's current value came from — one of `default`,
   * `configuration file`, `command line`, `session`, `client`,
   * `database`, `user`, `override`, etc.
   */
  source: string;
  bootVal: string | null;
  resetVal: string | null;
};

/** One row per installed Postgres extension, surfaced read-only. */
export type PgExtension = {
  name: string;
  version: string;
  schema: string;
  description: string | null;
};

/**
 * Aggregate server-info snapshot used by the Details sub-tab.
 * Populated only for Postgres connections via `loadServerDetails`.
 */
export type ServerDetails = {
  serverVersion: string;
  encoding: string;
  locale: string;
  timezone: string;
  settings: PgSetting[];
  extensions: PgExtension[];
};

export type ServerDetailsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

// Re-export domain types from their owning modules so the workspace
// state type (and slice files) can refer to them through one import
// path.
export type { ColumnChangeKind };
export type {
  SchemaMapAttrMode,
  SchemaMapPosition,
  SchemaMapPrefs,
  SchemaMapRouting,
  SchemaRelationships,
};

// ---------------------------------------------------------------------------
// Schema explorer + saved queries + history
// ---------------------------------------------------------------------------

export type SchemaExplorer = {
  name: string;
  tables: string[];
  views?: string[];
  materializedViews?: string[];
  sequences?: string[];
  foreignTables?: string[];
  functions?: string[];
  procedures?: string[];
  aggregateFunctions?: string[];
  types?: string[];
  domains?: string[];
  extensions?: string[];
  eventTriggers?: string[];
  roles?: string[];
  tablespaces?: string[];
};

export type PgObjectKind =
  | "schema"
  | "table"
  | "view"
  | "materialized-view"
  | "foreign-table"
  | "sequence"
  | "function"
  | "procedure"
  | "aggregate"
  | "type"
  | "domain"
  | "extension";

export type PgTypeClass = "enum" | "composite" | "range" | "multirange";

export type PgRoutineObjectKind = "function" | "procedure" | "aggregate";
export type PgScopedObjectKind = Exclude<
  PgObjectKind,
  "schema" | PgRoutineObjectKind
>;

/** Object identity is valid by construction inside the typed application. */
export type PgObjectRef =
  | { kind: "schema"; schema: null; name: string; identityArgs: null }
  | {
      kind: PgRoutineObjectKind;
      schema: string;
      name: string;
      identityArgs: string;
    }
  | {
      kind: PgScopedObjectKind;
      schema: string;
      name: string;
      identityArgs: null;
    };

export type PgCatalogEntry = {
  name: string;
  identityArgs?: string;
  comment?: string;
  typeClass?: PgTypeClass;
};

export type PgSchemaObjects = {
  name: string;
  tables: PgCatalogEntry[];
  views: PgCatalogEntry[];
  materializedViews: PgCatalogEntry[];
  foreignTables: PgCatalogEntry[];
  sequences: PgCatalogEntry[];
  functions: PgCatalogEntry[];
  procedures: PgCatalogEntry[];
  aggregates: PgCatalogEntry[];
  types: PgCatalogEntry[];
  domains: PgCatalogEntry[];
  extensions: PgCatalogEntry[];
};

export type PgCatalogTruncation = {
  schema: string | null;
  kind: string;
};

export type PgObjectCatalog = {
  schemas: PgSchemaObjects[];
  eventTriggers: PgCatalogEntry[];
  roles: PgCatalogEntry[];
  tablespaces: PgCatalogEntry[];
  truncated: PgCatalogTruncation[];
};

export type PgTypeAttribute = {
  name: string;
  dataType: string;
  nullable: boolean;
};

export type PgObjectFacts =
  | { kind: "schema" }
  | { kind: "table" }
  | { kind: "view"; definition: string }
  | { kind: "materializedView"; definition: string; populated: boolean }
  | { kind: "foreignTable"; server: string }
  | {
      kind: "sequence";
      dataType: string;
      start: string;
      increment: string;
      minValue: string;
      maxValue: string;
      cycle: boolean;
      cache: string;
      lastValue: string | null;
      ownedBy: string | null;
    }
  | {
      kind: "routine";
      language: string;
      returns: string | null;
      volatility: string | null;
      arguments: string;
      /** `prosrc`; the symbol name for C/internal routines, null for aggregates. */
      body: string | null;
      strict: boolean;
      securityDefiner: boolean;
      /** `safe` | `restricted` | `unsafe`; null for aggregates. */
      parallel: string | null;
    }
  | {
      kind: "type";
      class: PgTypeClass;
      enumLabels: string[] | null;
      attributes: PgTypeAttribute[] | null;
      subtype: string | null;
    }
  | {
      kind: "domain";
      baseType: string;
      notNull: boolean;
      defaultValue: string | null;
      checks: string[];
    }
  | { kind: "extension"; version: string; schema: string };

export type PgObjectDescription = {
  reference: PgObjectRef;
  owner: string | null;
  comment: string | null;
  definitionSql: string | null;
  facts: PgObjectFacts;
};

export type PgDropDependent = {
  objectType: string;
  identity: string;
  depth: number;
};

export type PgDropImpact = {
  dependents: PgDropDependent[];
  truncated: boolean;
};

export type PgDefaultValue =
  | { kind: "literal"; value: string }
  | { kind: "expression"; sql: string };

export type PgIdentity = "always" | "by-default";

export type PgNewColumnSpec = {
  name: string;
  dataType: string;
  nullable: boolean;
  default: PgDefaultValue | null;
  /** Identity columns must be NOT NULL and carry no default. */
  identity?: PgIdentity | null;
};

export type PgKeySpec = {
  name: string | null;
  columns: string[];
};

export type PgCheckSpec = {
  name: string | null;
  expression: string;
};

export type PgForeignKeySpec = {
  name: string | null;
  columns: string[];
  referencedSchema: string;
  referencedTable: string;
  referencedColumns: string[];
  onUpdate: PgReferentialAction;
  onDelete: PgReferentialAction;
  deferrable: boolean;
  initiallyDeferred: boolean;
};

export type PgVolatility = "immutable" | "stable" | "volatile";

export type PgParallelSafety = "safe" | "restricted" | "unsafe";

export type PgTriggerTiming = "before" | "after" | "instead-of";

export type PgTriggerEvent =
  | { kind: "insert" }
  | { kind: "update"; columns: string[] }
  | { kind: "delete" }
  | { kind: "truncate" };

export type PgTriggerLevel = "row" | "statement";

export type PgTriggerMode =
  | "enable"
  | "disable"
  | "enable-replica"
  | "enable-always";

export type PgGrantee = { kind: "public" } | { kind: "role"; name: string };

export type PgPolicyCommand = "all" | "select" | "insert" | "update" | "delete";

export type PgPrivilege =
  | "select"
  | "insert"
  | "update"
  | "delete"
  | "truncate"
  | "references"
  | "trigger"
  | "usage"
  | "create"
  | "execute"
  | "maintain";

export type PgReferentialAction =
  | "no-action"
  | "restrict"
  | "cascade"
  | "set-null"
  | "set-default";

export type PgIndexColumn = {
  expression: string;
  descending: boolean;
};

export type PgEnumPosition =
  | { kind: "before"; neighbor: string }
  | { kind: "after"; neighbor: string };

export type PgCommentTarget =
  | { kind: "object"; reference: PgObjectRef }
  | { kind: "column"; schema: string; table: string; column: string };

export type PgObjectOp =
  | { op: "createSchema"; name: string }
  | { op: "renameObject"; reference: PgObjectRef; newName: string }
  | { op: "dropObject"; reference: PgObjectRef; cascade: boolean }
  | { op: "setComment"; target: PgCommentTarget; comment: string | null }
  | {
      op: "addColumn";
      schema: string;
      table: string;
      column: PgNewColumnSpec;
    }
  | {
      op: "dropColumn";
      schema: string;
      table: string;
      name: string;
      cascade: boolean;
    }
  | {
      op: "renameColumn";
      schema: string;
      table: string;
      name: string;
      newName: string;
    }
  | {
      op: "alterColumnType";
      schema: string;
      table: string;
      name: string;
      newType: string;
      using: string | null;
    }
  | {
      op: "setColumnNullable";
      schema: string;
      table: string;
      name: string;
      nullable: boolean;
    }
  | {
      op: "setColumnDefault";
      schema: string;
      table: string;
      name: string;
      default: PgDefaultValue | null;
    }
  | {
      op: "addPrimaryKey";
      schema: string;
      table: string;
      name: string | null;
      columns: string[];
    }
  | {
      op: "addUnique";
      schema: string;
      table: string;
      name: string | null;
      columns: string[];
    }
  | {
      op: "addForeignKey";
      schema: string;
      table: string;
      name: string | null;
      columns: string[];
      referencedSchema: string;
      referencedTable: string;
      referencedColumns: string[];
      onUpdate: PgReferentialAction;
      onDelete: PgReferentialAction;
      deferrable: boolean;
      initiallyDeferred: boolean;
      notValid: boolean;
    }
  | {
      op: "addCheck";
      schema: string;
      table: string;
      name: string | null;
      expression: string;
      notValid: boolean;
    }
  | {
      op: "dropConstraint";
      schema: string;
      table: string;
      name: string;
      cascade: boolean;
    }
  | {
      op: "createIndex";
      schema: string;
      table: string;
      name: string | null;
      unique: boolean;
      method: string;
      columns: PgIndexColumn[];
      include: string[];
      wherePredicate: string | null;
      concurrently: boolean;
    }
  | {
      op: "dropIndex";
      schema: string;
      name: string;
      concurrently: boolean;
      cascade: boolean;
    }
  | {
      op: "createView";
      schema: string;
      name: string;
      orReplace: boolean;
      sqlBody: string;
    }
  | {
      op: "createMaterializedView";
      schema: string;
      name: string;
      sqlBody: string;
      withData: boolean;
    }
  | {
      op: "createSequence";
      schema: string;
      name: string;
      dataType: string | null;
      start: string | null;
      increment: string | null;
      minValue: string | null;
      maxValue: string | null;
      cycle: boolean | null;
      cache: string | null;
    }
  | {
      op: "alterSequence";
      schema: string;
      name: string;
      restartWith: string | null;
      incrementBy: string | null;
      minValue: string | null;
      maxValue: string | null;
      cycle: boolean | null;
      cache: string | null;
    }
  | { op: "createEnum"; schema: string; name: string; labels: string[] }
  | {
      op: "addEnumValue";
      schema: string;
      name: string;
      value: string;
      position: PgEnumPosition | null;
    }
  | {
      op: "renameEnumValue";
      schema: string;
      name: string;
      from: string;
      to: string;
    }
  | {
      op: "createTable";
      schema: string;
      name: string;
      columns: PgNewColumnSpec[];
      primaryKey: PgKeySpec | null;
      uniques: PgKeySpec[];
      checks: PgCheckSpec[];
      foreignKeys: PgForeignKeySpec[];
      unlogged: boolean;
      ifNotExists: boolean;
    }
  | {
      op: "createFunction";
      schema: string;
      name: string;
      orReplace: boolean;
      /** Signature fragment, e.g. `a integer, b text DEFAULT 'x'`. */
      arguments: string;
      /** Return clause fragment, e.g. `trigger`, `SETOF integer`. */
      returns: string;
      language: string;
      /** Opaque; the backend seals it in a dollar quote. */
      body: string;
      volatility: PgVolatility;
      strict: boolean;
      securityDefiner: boolean;
      parallel: PgParallelSafety | null;
    }
  | {
      op: "createProcedure";
      schema: string;
      name: string;
      orReplace: boolean;
      arguments: string;
      language: string;
      body: string;
      securityDefiner: boolean;
    }
  | {
      op: "createTrigger";
      schema: string;
      table: string;
      name: string;
      timing: PgTriggerTiming;
      events: PgTriggerEvent[];
      forEach: PgTriggerLevel;
      when: string | null;
      functionSchema: string;
      functionName: string;
      arguments: string[];
      orReplace: boolean;
    }
  | {
      op: "dropTrigger";
      schema: string;
      table: string;
      name: string;
      cascade: boolean;
    }
  | {
      op: "setTriggerEnabled";
      schema: string;
      table: string;
      name: string;
      mode: PgTriggerMode;
    }
  | {
      op: "setRowLevelSecurity";
      schema: string;
      table: string;
      enabled: boolean;
      force: boolean | null;
    }
  | {
      op: "createPolicy";
      schema: string;
      table: string;
      name: string;
      permissive: boolean;
      command: PgPolicyCommand;
      roles: PgGrantee[];
      using: string | null;
      withCheck: string | null;
    }
  | { op: "dropPolicy"; schema: string; table: string; name: string }
  | {
      op: "grantPrivileges";
      target: PgObjectRef;
      privileges: PgPrivilege[];
      allPrivileges: boolean;
      grantee: PgGrantee;
      withGrantOption: boolean;
    }
  | {
      op: "revokePrivileges";
      target: PgObjectRef;
      privileges: PgPrivilege[];
      allPrivileges: boolean;
      grantee: PgGrantee;
      grantOptionFor: boolean;
      cascade: boolean;
    };

export type PlannedStatement = {
  sql: string;
  summary: string;
  destructive: boolean;
  transactional: boolean;
};

export type StatementGroup =
  | { kind: "atomic"; statementIndexes: number[] }
  | { kind: "standalone"; statementIndex: number };

export type DdlPlanPreview = {
  statements: PlannedStatement[];
  groups: StatementGroup[];
};

export type DdlStatementSummary = {
  index: number;
  summary: string;
  destructive: boolean;
  transactional: boolean;
};

export type DdlApplyResult = {
  appliedStatements: number;
  runtimeMs: number;
};

export type DdlResidue = {
  kind: "invalidIndex";
  schema: string;
  name: string;
};

export type PgObjectError =
  | { kind: "unsupportedEngine"; engine: string }
  | { kind: "objectNotFound"; reference: PgObjectRef }
  | { kind: "invalidOp"; opIndex: number; reason: string }
  | { kind: "policyBlocked"; reason: string }
  | { kind: "policyNeedsConfirmation"; statements: DdlStatementSummary[] }
  | { kind: "connection"; message: string }
  | {
      kind: "lockTimeout";
      statementIndex: number;
      appliedStatements: number;
      residue?: DdlResidue;
    }
  | {
      kind: "database";
      statementIndex?: number;
      code: string | null;
      message: string;
      position: number | null;
      appliedStatements: number;
      residue?: DdlResidue;
    };

export type SavedQuery = {
  id: string;
  name: string;
  body: string;
  /** `null` = saved query is not pinned to a specific connection. */
  connectionId: string | null;
  isFavorite: boolean;
  /** Reserved for future cloud-sync. Local writes leave this null. */
  ownerId?: string | null;
  createdAt: string;
  updatedAt: string;
};

export type SavedQueriesStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

export type QueryHistoryEntry = {
  id: string;
  sql: string;
  connectionId: string;
  connectionName: string;
  database: string;
  engine: DatabaseEngine;
  status: "success" | "error";
  errorMessage?: string;
  runtimeMs: number;
  rowCount?: number;
  startedAt: string;
};

// ---------------------------------------------------------------------------
// Workspace tabs
// ---------------------------------------------------------------------------

export type WorkspaceTabKind =
  | "pg-tools"
  | "table"
  | "table-designer"
  | "query"
  | "object"
  | "key"
  | "cli"
  | "pubsub"
  | "server";

/** Persisted editor caret/selection for a query tab (Plan 009). All
 *  members are 1-based Monaco positions; the restore path clamps to
 *  the model, so stale values degrade instead of erroring. */
export type TabCaret = {
  line: number;
  column: number;
  anchorLine?: number;
  anchorColumn?: number;
};

export type TableDesignerColumnDraft = {
  id: string;
  name: string;
  dataType: string;
  nullable: boolean;
  identity: "none" | PgIdentity;
  defaultKind: "none" | "literal" | "expression";
  defaultValue: string;
  comment: string;
};

export type TableDesignerIndexDraft = {
  id: string;
  name: string;
  columns: string[];
  unique: boolean;
  method: string;
  include: string[];
  wherePredicate: string;
  concurrently: boolean;
};

export type TableDesignerKeyDraft = PgKeySpec & { id: string };
export type TableDesignerCheckDraft = PgCheckSpec & { id: string };
export type TableDesignerForeignKeyDraft = PgForeignKeySpec & { id: string };

/** Complete, serializable form state owned by a table-designer tab. */
export type TableDesignerDraft = {
  schema: string;
  name: string;
  comment: string;
  columns: TableDesignerColumnDraft[];
  primaryKey: PgKeySpec | null;
  uniques: TableDesignerKeyDraft[];
  checks: TableDesignerCheckDraft[];
  foreignKeys: TableDesignerForeignKeyDraft[];
  indexes: TableDesignerIndexDraft[];
  unlogged: boolean;
};

export type WorkspaceTab = {
  /** Transient backup/restore tabs never persist paths or job state. */
  toolOperation?: "backup" | "restore";
  id: string;
  kind: WorkspaceTabKind;
  label: string;
  connectionId: string;
  schema: string;
  table?: string;
  /** PostgreSQL object tabs: overload-safe, canonical object identity. */
  objectRef?: PgObjectRef;
  /** Query tabs opened for a relation: schema-qualified dedupe identity. */
  relationRef?: { schema: string; name: string };
  query?: string;
  /** Query tabs only: last known caret/selection, session-persisted. */
  caret?: TabCaret;
  /** Table-designer tabs only: complete form draft, session-persisted. */
  tableDesignerDraft?: TableDesignerDraft;
  /** Table-designer tabs only: runtime guard while reviewed DDL is applying. */
  tableDesignerApplying?: boolean;
  lastRun?: string;
  isDirty?: boolean;
  /** Pinned tabs sit leftmost at icon width and are excluded from
   *  Close Others / Close All (DESIGN-SYSTEM §4.4). */
  pinned?: boolean;
  /** Redis `key` tab: the inspected key name (under `dbNumber`). */
  redisKey?: string;
  /** Redis `key` tab: scoped DB number at open time. */
  redisDbNumber?: number;
};

export type TablePreviewData = {
  columns: string[];
  rows: string[][];
  rowCount: string;
  primaryKey: string;
  size: string;
  lastVacuum: string;
};

export type QueryPreviewData = {
  columns: string[];
  rows: string[][];
  runtime: string;
  rowCount: string;
  cache: string;
};

export type ActiveView = "workspace" | "settings";

/**
 * Sub-tab inside the unified Settings page. The Settings view's tab
 * rail switches between these; the active id persists on
 * `WorkspaceTabsSlice.settingsTab` so users land on the same tab they
 * left and so other entry points (sidebar "manage connections" cog,
 * future deep links) can open Settings on a specific tab.
 */
export type SettingsTab =
  | "general"
  | "connections"
  | "bastions"
  | "local-databases"
  | "security"
  | "about";

/**
 * Sub-tab inside a relational connection's Overview surface. The
 * Overview header's tab nav switches the body region between these
 * views; selection is persisted per connection on
 * `ConnectionsSlice.connectionOverviewTab` so a user landing back on
 * a connection lands on the same sub-tab they left.
 *
 * `"overview"` keeps the existing dashboard cards. The other ids back
 * the Phase 1 deep views (see `docs/design/PHASES.md` — Phase 1).
 */
export type OverviewTabId =
  | "overview"
  | "tables"
  | "schemas"
  | "schema-map"
  | "query-history"
  | "admin"
  | "compare"
  | "details"
  | "settings";

// ---------------------------------------------------------------------------
// AppStoreState — the full store shape
// ---------------------------------------------------------------------------

import type { BastionsSlice } from "./bastions";
import type { ConnectionsSlice } from "./connections";
import type { ConsoleSlice } from "./console";
import type { CredentialsSlice } from "./credentials";
import type { KeyValuePubSubSlice } from "./keyvalue-pubsub";
import type { KeyValueWorkspaceSlice } from "./keyvalue-workspace";
import type { ManagedServersSlice } from "./managed-servers";
import type { MutationDraftsSlice } from "./mutation-drafts";
import type { PgObjectsSlice } from "./pg-objects";
import type { QuerySessionsSlice } from "./query-sessions";
import type { RelationalQueriesSlice } from "./relational-queries";
import type { RelationalTablesSlice } from "./relational-tables";
import type { TableBrowseSlice } from "./table-browse";
import type { WorkspaceTabsSlice } from "./workspace-tabs";

/**
 * The complete shape of the Zustand store — the intersection of every
 * slice. Each slice's `StateCreator` factory types its `set`/`get`
 * against this shape so cross-slice `get()` calls typecheck.
 *
 * Slices own their own fields and actions; this alias just composes
 * them. Type-only circular import (each slice file imports
 * `AppStoreState` from here, and we import each slice's type from
 * those files) is resolved by `import type`.
 */
export type AppStoreState = ConnectionsSlice &
  BastionsSlice &
  ManagedServersSlice &
  CredentialsSlice &
  WorkspaceTabsSlice &
  PgObjectsSlice &
  RelationalTablesSlice &
  MutationDraftsSlice &
  QuerySessionsSlice &
  TableBrowseSlice &
  RelationalQueriesSlice &
  KeyValueWorkspaceSlice &
  KeyValuePubSubSlice &
  ConsoleSlice;

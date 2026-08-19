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

import type { ColumnChangeKind, PendingChange } from "@/lib/ddl";
import type {
  SchemaMapAttrMode,
  SchemaMapPosition,
  SchemaMapPrefs,
  SchemaMapRouting,
  SchemaRelationships,
} from "@/lib/schema-graph";
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
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
};

/**
 * Per-connection driver/session knobs, persisted as a JSON blob on
 * the Postgres connection record. See ADR-0013. Each field is
 * optional — `undefined` means "use the server default".
 *
 * `connectTimeoutMs` and `keepaliveSeconds` are reserved on the
 * struct but not yet applied at connect time (sqlx 0.8 doesn't
 * expose those setters directly).
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
  /** TLS upgrade on the wire protocol. PG/MySQL only — distinct from
   *  ClickHouse `useHttps` and Redis `useTls`. */
  ssl: boolean;
  /** Optional driver/session knobs applied after every connect.
   *  See ADR-0013. Missing or empty fields fall back to PG defaults. */
  driverOptions?: PgDriverOptions;
  sshTunnel?: SshTunnelConfig;
};
export type MySqlStoredConnection = ConnectionCommon & {
  engine: "MySQL";
  ssl: boolean;
  sshTunnel?: SshTunnelConfig;
};
export type SqliteStoredConnection = ConnectionCommon & {
  engine: "SQLite";
};
export type ClickHouseStoredConnection = ConnectionCommon & {
  engine: "ClickHouse";
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
  | { kind: "timeout"; operation: string }
  | ({ kind: "database" } & QueryDatabaseError);
export type QueryNotice = { severity: string; message: string };
export type QueryResultSet = {
  index: number;
  columns: Array<string | null>;
  rows: Array<Array<string | null>>;
  rowCount: number;
  partial: boolean;
  completed: boolean;
};
export type QueryExecutionTombstone = {
  status: string;
  resultCount: number;
  rowCount: number;
  affectedCount: number;
  noticeCount: number;
  omittedCount: number;
  runtimeMs: number;
  releasedBytes: number;
  completedAt: string;
  reason: "globalBudget";
};
export type QueryExecution = {
  id: string;
  status: "running" | "completed" | "failed" | "cancelled" | "lost";
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
  generation: number;
  nextSequence: number;
  transaction: QueryTransactionSnapshot;
  execution: QueryExecution | null;
  lastViewedAt: number;
  budgetOwners: Array<{ tabId: string; label: string; retainedBytes: number }>;
  state: "opening" | "open" | "lost" | "closed";
  error: QuerySessionError | null;
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
      preview: QueryPreviewData;
    }
  | { kind: "failed"; reason: string }
  | { kind: "cancelled" }
  | { kind: "noop" };

export type TableLoadStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

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

export type StructureCapabilities = {
  columns: boolean;
  primaryKey: boolean;
  foreignKeys: boolean;
  indexes: boolean;
  constraints: boolean;
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
export type { ColumnChangeKind, PendingChange };
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
  | "table"
  | "query"
  | "key"
  | "cli"
  | "pubsub"
  | "server";

export type WorkspaceTab = {
  id: string;
  kind: WorkspaceTabKind;
  label: string;
  connectionId: string;
  schema: string;
  table?: string;
  query?: string;
  lastRun?: string;
  isDirty?: boolean;
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
import type { CredentialsSlice } from "./credentials";
import type { KeyValuePubSubSlice } from "./keyvalue-pubsub";
import type { KeyValueWorkspaceSlice } from "./keyvalue-workspace";
import type { ManagedServersSlice } from "./managed-servers";
import type { RelationalQueriesSlice } from "./relational-queries";
import type { RelationalTablesSlice } from "./relational-tables";
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
  RelationalTablesSlice &
  RelationalQueriesSlice &
  KeyValueWorkspaceSlice &
  KeyValuePubSubSlice;

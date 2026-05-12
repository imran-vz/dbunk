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

import type { PendingChange } from "@/lib/ddl";
import type { SchemaRelationships } from "@/lib/schema-graph";

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

export type AppSettingsSnapshot = {
  onboardingCompleted: boolean;
  credentialStorageMode: CredentialStorageMode | null;
  credentialState: CredentialState;
  configDir: string;
};

export type AppSettingsStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "ready" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Connection records
// ---------------------------------------------------------------------------

export type StoredConnection = {
  id: string;
  name: string;
  database: string;
  engine: DatabaseEngine;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
  /** ClickHouse-only: connect over HTTPS instead of HTTP. */
  useHttps?: boolean;
  /** ClickHouse-only: URL path prefix for proxied deployments (e.g. /clickhouse). */
  urlPath?: string;
  /** Redis-only: which numbered DB (0–15 on standalone). Defaults to 0. */
  dbNumber?: number;
  /** Redis-only: connect over TLS (rediss://). */
  useTls?: boolean;
  /** Redis-only: verify the TLS certificate. Only meaningful when useTls is true. Default true. */
  verifyTlsCert?: boolean;
};

export type Connection = {
  id: string;
  name: string;
  database: string;
  status: "Connected" | "Read only" | "Disconnected";
  engine: DatabaseEngine;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  latency: string;
  lastSync: string;
  /** ISO-8601 timestamp of the most recent successful query/connect. */
  lastActivityAt?: string;
  errorMessage?: string;
  /** ClickHouse-only: connect over HTTPS instead of HTTP. */
  useHttps?: boolean;
  /** ClickHouse-only: URL path prefix for proxied deployments. */
  urlPath?: string;
  /** Redis-only: which numbered DB (0–15 on standalone). */
  dbNumber?: number;
  /** Redis-only: connect over TLS (rediss://). */
  useTls?: boolean;
  /** Redis-only: verify the TLS certificate when useTls is on. */
  verifyTlsCert?: boolean;
};

// ---------------------------------------------------------------------------
// Per-tab status shapes
// ---------------------------------------------------------------------------

export type QueryStatus =
  | { state: "idle" }
  | { state: "running" }
  | { state: "success"; runtimeMs?: number }
  | { state: "error"; error: string };

export type TableLoadStatus =
  | { state: "idle" }
  | { state: "loading" }
  | { state: "success" }
  | { state: "error"; error: string };

// ---------------------------------------------------------------------------
// Table data + structure
// ---------------------------------------------------------------------------

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

export type ColumnInfo = {
  name: string;
  dataType: string;
  nullable: boolean;
  defaultValue: string | null;
  isPrimaryKey: boolean;
  ordinalPosition: number;
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

export type StructureCommitStatus =
  | { state: "idle" }
  | { state: "running" }
  | { state: "success"; runtimeMs?: number }
  | { state: "error"; error: string };

export type TableEditsCommitStatus =
  | { state: "idle" }
  | { state: "running" }
  | {
      state: "queued";
      database: string;
      table: string;
      mutationIds: string[];
      runtimeMs: number;
    }
  | { state: "success"; rowsAffected: number; runtimeMs: number }
  | { state: "error"; error: string };

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

// Re-export PendingChange + SchemaRelationships from their owning modules so
// the workspace state type can refer to them through one import path.
export type { PendingChange, SchemaRelationships };

// ---------------------------------------------------------------------------
// Schema explorer + saved queries + history
// ---------------------------------------------------------------------------

export type SchemaExplorer = {
  name: string;
  tables: string[];
  views?: string[];
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

export type ActiveView = "workspace" | "connections" | "settings";

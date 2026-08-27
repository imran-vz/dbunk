/**
 * Pure helpers for `ConnectionForm`: schema, builders, defaults. Kept
 * away from the React tree so they're unit-testable per engine branch
 * without spinning up a form. The per-engine projection lives in
 * `buildStoredConnectionFromForm`, split across one helper per variant
 * to keep each branch trivial (no nested switch + no shared common
 * spread interleaved with per-engine fields).
 */

import * as z from "zod";

import { CONNECTION_COLORS, isConnectionColor } from "@/lib/connection-colors";
import type {
  ClickHouseStoredConnection,
  Connection,
  ConnectionEnvironment,
  MySqlStoredConnection,
  PgDriverOptions,
  PgStoredConnection,
  RedisStoredConnection,
  SafeMode,
  SqliteStoredConnection,
  SshTunnelConfig,
  StoredConnection,
} from "@/lib/store";
import {
  PG_TLS_MODES,
  type PgTlsMode,
  type PgTlsOptions,
} from "@/lib/store/types";

export const connectionSchema = z.object({
  name: z.string().min(1, "Connection name is required"),
  engine: z.enum(["PostgreSQL", "MySQL", "ClickHouse", "SQLite", "Redis"]),
  host: z.string().optional(),
  database: z.string().optional(),
  port: z.number().int().optional(),
  user: z.string().optional(),
  password: z.string().optional(),
  role: z.string().optional(),
  environment: z.enum(["development", "test", "staging", "production"]),
  safeMode: z.enum(["inherit", "disabled", "protected", "strict"]),
  folder: z.string().max(120).optional(),
  isFavorite: z.boolean().optional(),
  color: z.enum(CONNECTION_COLORS).optional(),
  ssl: z.boolean().optional(),
  useHttps: z.boolean().optional(),
  urlPath: z.string().optional(),
  dbNumber: z.number().int().min(0).max(15).optional(),
  useTls: z.boolean().optional(),
  verifyTlsCert: z.boolean().optional(),
  readOnly: z.boolean().optional(),
  sshTunnelEnabled: z.boolean().optional(),
  sshTunnelBastionServerId: z.string().optional(),
  sshTunnelLocalBindHost: z.string().optional(),
  sshTunnelLocalPort: z.number().int().optional(),
  sshTunnelCompression: z.boolean().optional(),
  sshTunnelKeepaliveIntervalSeconds: z.number().int().optional(),
  sshTunnelKeepaliveWantReply: z.boolean().optional(),
  sshTunnelJumpChain: z.array(z.string()).optional(),
  sshTunnelProxyCommand: z.string().optional(),
  statementTimeoutMs: z.number().int().optional(),
  idleInTransactionTimeoutMs: z.number().int().optional(),
  connectTimeoutMs: z.number().int().optional(),
  keepaliveSeconds: z.number().int().optional(),
  defaultSearchPath: z.string().optional(),
  defaultRole: z.string().optional(),
  // PostgreSQL TLS (ADR-0025) as per-field values; `tlsOptionsFromForm`
  // folds them back into the stored blob.
  tlsMode: z.enum(PG_TLS_MODES).optional(),
  tlsRootCertPath: z.string().optional(),
  tlsClientCertPath: z.string().optional(),
  tlsClientKeyPath: z.string().optional(),
  tlsServerName: z.string().optional(),
});

export type ConnectionFormData = z.infer<typeof connectionSchema>;

type TlsFormDefaults = Pick<
  ConnectionFormData,
  | "tlsMode"
  | "tlsRootCertPath"
  | "tlsClientCertPath"
  | "tlsClientKeyPath"
  | "tlsServerName"
>;

const TLS_NEUTRAL_DEFAULTS: TlsFormDefaults = {
  tlsMode: "prefer",
  tlsRootCertPath: "",
  tlsClientCertPath: "",
  tlsClientKeyPath: "",
  tlsServerName: "",
};

export const EMPTY_NEW_DEFAULTS: ConnectionFormData = {
  name: "",
  engine: "PostgreSQL",
  host: "",
  database: "",
  port: 5432,
  user: "",
  password: "",
  role: "read/write",
  environment: "development",
  safeMode: "inherit",
  folder: "",
  isFavorite: false,
  color: undefined,
  ssl: true,
  useHttps: false,
  urlPath: "",
  dbNumber: 0,
  useTls: false,
  verifyTlsCert: true,
  readOnly: false,
  sshTunnelEnabled: false,
  sshTunnelBastionServerId: "",
  sshTunnelLocalBindHost: "127.0.0.1",
  sshTunnelLocalPort: undefined,
  sshTunnelCompression: false,
  sshTunnelKeepaliveIntervalSeconds: undefined,
  sshTunnelKeepaliveWantReply: true,
  sshTunnelJumpChain: [],
  sshTunnelProxyCommand: "",
  statementTimeoutMs: undefined,
  idleInTransactionTimeoutMs: undefined,
  connectTimeoutMs: undefined,
  keepaliveSeconds: undefined,
  defaultSearchPath: "",
  defaultRole: "",
  ...TLS_NEUTRAL_DEFAULTS,
};

// oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
type CommonShape = {
  id: string;
  name: string;
  database: string;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
  environment: ConnectionEnvironment;
  safeMode: SafeMode;
  readOnly: boolean;
  folder: string;
  isFavorite: boolean;
  color?: import("@/lib/connection-colors").ConnectionColor;
};

// oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
function commonFromForm(value: ConnectionFormData, id: string): CommonShape {
  return {
    id,
    name: value.name,
    database: value.database ?? "",
    host: value.host ?? "",
    port: value.port ?? 0,
    user: value.user ?? "",
    password: value.password ?? "",
    role: value.role || "read/write",
    environment: value.environment ?? "development",
    safeMode: value.safeMode ?? "inherit",
    readOnly: value.readOnly ?? false,
    folder: value.folder?.trim() ?? "",
    isFavorite: value.isFavorite ?? false,
    ...(value.color ? { color: value.color } : null),
  };
}

function buildPg(
  value: ConnectionFormData,
  // oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
  common: CommonShape,
): PgStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  const driverOptions = driverOptionsFromForm(value);
  const tlsOptions = tlsOptionsFromForm(value);
  return {
    ...common,
    engine: "PostgreSQL",
    ssl: value.tlsMode ? value.tlsMode !== "disable" : (value.ssl ?? true),
    ...(tlsOptions ? { tlsOptions } : null),
    ...(driverOptions ? { driverOptions } : null),
    ...(sshTunnel ? { sshTunnel } : null),
  };
}

/**
 * Split the comma-separated search-path text into the schema list the
 * backend stores. Blank entries are dropped rather than rejected —
 * `validateConnection` already surfaces them as a form issue, and this
 * builder must stay total for the values it does receive.
 */
export function parseSearchPath(raw: string | undefined): string[] {
  return (raw ?? "")
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
}

/** Inverse of `parseSearchPath`, for hydrating the text input. */
export function formatSearchPath(path: string[] | undefined): string {
  return (path ?? []).join(", ");
}

/**
 * Project the ADR-0013 knobs out of form state. Returns `undefined`
 * when every knob is empty so the connection record stays free of an
 * all-`undefined` blob — `driver_options` then persists as SQL NULL
 * and the backend skips the post-connect `SET` round-trip entirely.
 */
export function driverOptionsFromForm(
  value: ConnectionFormData,
): PgDriverOptions | undefined {
  const searchPath = parseSearchPath(value.defaultSearchPath);
  const defaultRole = value.defaultRole?.trim();
  const options: PgDriverOptions = {
    ...(value.statementTimeoutMs !== undefined
      ? { statementTimeoutMs: value.statementTimeoutMs }
      : null),
    ...(value.idleInTransactionTimeoutMs !== undefined
      ? { idleInTransactionTimeoutMs: value.idleInTransactionTimeoutMs }
      : null),
    ...(value.connectTimeoutMs !== undefined
      ? { connectTimeoutMs: value.connectTimeoutMs }
      : null),
    ...(value.keepaliveSeconds !== undefined
      ? { keepaliveSeconds: value.keepaliveSeconds }
      : null),
    ...(searchPath.length > 0 ? { defaultSearchPath: searchPath } : null),
    ...(defaultRole ? { defaultRole } : null),
  };
  return Object.keys(options).length > 0 ? options : undefined;
}

/** Whether a mode checks the server certificate at all. */
export function tlsModeVerifies(mode: PgTlsMode): boolean {
  return mode === "verify-ca" || mode === "verify-full";
}

/**
 * Which optional TLS fields a mode reads — mirrors the resolver in
 * `src-tauri/src/postgres/tls.rs`. The form shows exactly these, and
 * `tlsOptionsFromForm` persists exactly these, so a path typed under
 * one mode never survives invisibly into another.
 */
export function tlsModeFields(mode: PgTlsMode) {
  return {
    rootCert: tlsModeVerifies(mode),
    clientCert: mode !== "disable",
    serverName: mode === "verify-full",
  };
}

/**
 * Fold the per-field TLS values into the ADR-0025 blob. Returns
 * `undefined` for the default (`prefer`, no paths) so the record stays
 * free of a blob that says nothing. `disable` is emitted even alone: a
 * legacy `ssl: false` record hydrates as `disable` and must save back
 * as `disable`, not snap to `prefer`.
 */
export function tlsOptionsFromForm(
  value: ConnectionFormData,
): PgTlsOptions | undefined {
  const mode = value.tlsMode ?? "prefer";
  const fields = tlsModeFields(mode);
  const rootCertPath = value.tlsRootCertPath?.trim();
  const clientCertPath = value.tlsClientCertPath?.trim();
  const clientKeyPath = value.tlsClientKeyPath?.trim();
  const serverName = value.tlsServerName?.trim();
  const options: PgTlsOptions = {
    mode,
    ...(fields.rootCert && rootCertPath ? { rootCertPath } : null),
    ...(fields.clientCert && clientCertPath ? { clientCertPath } : null),
    ...(fields.clientCert && clientKeyPath ? { clientKeyPath } : null),
    ...(fields.serverName && serverName ? { serverName } : null),
  };
  if (mode === "prefer" && Object.keys(options).length === 1) {
    return undefined;
  }
  return options;
}

function buildMySql(
  value: ConnectionFormData,
  // oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
  common: CommonShape,
): MySqlStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  return {
    ...common,
    engine: "MySQL",
    ssl: value.ssl ?? true,
    ...(sshTunnel ? { sshTunnel } : null),
  };
}

// oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
function buildSqlite(common: CommonShape): SqliteStoredConnection {
  return { ...common, engine: "SQLite" };
}

function buildClickHouse(
  value: ConnectionFormData,
  // oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
  common: CommonShape,
): ClickHouseStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  return {
    ...common,
    engine: "ClickHouse",
    useHttps: value.useHttps ?? false,
    urlPath: value.urlPath ?? "",
    ...(sshTunnel ? { sshTunnel } : null),
  };
}

function buildRedis(
  value: ConnectionFormData,
  // oxlint-disable-next-line anti-slop/no-shape-in-symbol-names -- The value is handled at a typed library or domain boundary here.
  common: CommonShape,
): RedisStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  return {
    ...common,
    engine: "Redis",
    dbNumber: value.dbNumber ?? 0,
    useTls: value.useTls ?? false,
    verifyTlsCert: value.verifyTlsCert ?? true,
    ...(sshTunnel ? { sshTunnel } : null),
  };
}

function tunnelFromForm(
  value: ConnectionFormData,
): SshTunnelConfig | undefined {
  if (!value.sshTunnelEnabled) {
    return undefined;
  }
  const bastionServerId = value.sshTunnelBastionServerId?.trim();
  const jumpChain = (value.sshTunnelJumpChain ?? [])
    .map((bastionId) => bastionId.trim())
    .filter(Boolean);
  const proxyCommand = value.sshTunnelProxyCommand?.trim();
  return {
    enabled: true,
    ...(bastionServerId ? { bastionServerId } : null),
    ...(value.sshTunnelLocalBindHost?.trim()
      ? { localBindHost: value.sshTunnelLocalBindHost.trim() }
      : null),
    ...(value.sshTunnelLocalPort
      ? { localPort: value.sshTunnelLocalPort }
      : null),
    ...(value.sshTunnelCompression ? { compression: true } : null),
    ...(value.sshTunnelKeepaliveIntervalSeconds
      ? { keepaliveIntervalSeconds: value.sshTunnelKeepaliveIntervalSeconds }
      : null),
    ...(value.sshTunnelKeepaliveWantReply === false
      ? { keepaliveWantReply: false }
      : null),
    ...(jumpChain.length > 0 ? { jumpChain } : null),
    ...(proxyCommand ? { proxyCommand } : null),
  };
}

/**
 * Project form values into the right `StoredConnection` variant. The
 * switch on `engine` is the single construction site for the wire
 * shape — Slice 4 collapses the previously-duplicated builders in
 * new-connection-form + edit-connection-dialog into this one place.
 */
export function buildStoredConnectionFromForm(
  value: ConnectionFormData,
  id: string,
): StoredConnection {
  const common = commonFromForm(value, id);
  switch (value.engine) {
    case "PostgreSQL":
      return buildPg(value, common);
    case "MySQL":
      return buildMySql(value, common);
    case "SQLite":
      return buildSqlite(common);
    case "ClickHouse":
      return buildClickHouse(value, common);
    case "Redis":
      return buildRedis(value, common);
  }
}

export function buildConnectionFromForm(
  value: ConnectionFormData,
  id: string,
  runtime: {
    status: Connection["status"];
    latency: string;
    errorMessage?: string;
    lastActivityAt?: string;
  },
): Connection {
  return { ...buildStoredConnectionFromForm(value, id), ...runtime };
}

export function defaultValuesFromConnection(
  connection: Connection,
): ConnectionFormData {
  const common = {
    name: connection.name,
    engine: connection.engine,
    host: connection.host,
    database: connection.database,
    port: connection.port || 5432,
    user: connection.user,
    password: "",
    role: connection.role,
    environment: connection.environment ?? "development",
    safeMode: connection.safeMode ?? "inherit",
    folder: connection.folder ?? "",
    isFavorite: connection.isFavorite ?? false,
    // The backend stores color opaquely — guard so an unknown token
    // hydrates as "no color" instead of failing schema validation.
    color: isConnectionColor(connection.color) ? connection.color : undefined,
  };
  const tunnel =
    connection.engine === "SQLite" ? undefined : connection.sshTunnel;
  const tunnelDefaults = {
    sshTunnelEnabled: tunnel?.enabled ?? false,
    sshTunnelBastionServerId: tunnel?.bastionServerId ?? "",
    sshTunnelLocalBindHost: tunnel?.localBindHost ?? "127.0.0.1",
    sshTunnelLocalPort: tunnel?.localPort,
    sshTunnelCompression: tunnel?.compression ?? false,
    sshTunnelKeepaliveIntervalSeconds: tunnel?.keepaliveIntervalSeconds,
    sshTunnelKeepaliveWantReply: tunnel?.keepaliveWantReply ?? true,
    sshTunnelJumpChain: tunnel?.jumpChain ?? [],
    sshTunnelProxyCommand: tunnel?.proxyCommand ?? "",
  };
  // Only PG carries driver options (ADR-0013); every other variant
  // hydrates the neutral blank so the shared form state stays total.
  const driverDefaults = driverDefaultsFromConnection(connection);
  const tlsDefaults = tlsDefaultsFromConnection(connection);
  switch (connection.engine) {
    case "PostgreSQL":
    case "MySQL":
      return {
        ...common,
        ...tunnelDefaults,
        ...driverDefaults,
        ...tlsDefaults,
        ssl: connection.ssl,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: connection.readOnly ?? false,
      };
    case "SQLite":
      return {
        ...common,
        ...tunnelDefaults,
        ...driverDefaults,
        ...tlsDefaults,
        ssl: true,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: connection.readOnly ?? false,
      };
    case "ClickHouse":
      return {
        ...common,
        ...tunnelDefaults,
        ...driverDefaults,
        ...tlsDefaults,
        ssl: true,
        useHttps: connection.useHttps,
        urlPath: connection.urlPath,
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: connection.readOnly ?? false,
      };
    case "Redis":
      return {
        ...common,
        ...tunnelDefaults,
        ...driverDefaults,
        ...tlsDefaults,
        ssl: true,
        useHttps: false,
        urlPath: "",
        dbNumber: connection.dbNumber,
        useTls: connection.useTls,
        verifyTlsCert: connection.verifyTlsCert,
        readOnly: connection.readOnly,
      };
  }
}

type DriverFormDefaults = Pick<
  ConnectionFormData,
  | "statementTimeoutMs"
  | "idleInTransactionTimeoutMs"
  | "connectTimeoutMs"
  | "keepaliveSeconds"
  | "defaultSearchPath"
  | "defaultRole"
>;

function driverDefaultsFromConnection(
  connection: Connection,
): DriverFormDefaults {
  const options =
    connection.engine === "PostgreSQL" ? connection.driverOptions : undefined;
  return {
    statementTimeoutMs: options?.statementTimeoutMs,
    idleInTransactionTimeoutMs: options?.idleInTransactionTimeoutMs,
    connectTimeoutMs: options?.connectTimeoutMs,
    keepaliveSeconds: options?.keepaliveSeconds,
    defaultSearchPath: formatSearchPath(options?.defaultSearchPath),
    defaultRole: options?.defaultRole ?? "",
  };
}

function tlsDefaultsFromConnection(connection: Connection): TlsFormDefaults {
  if (connection.engine !== "PostgreSQL") {
    return TLS_NEUTRAL_DEFAULTS;
  }
  const options = connection.tlsOptions;
  return {
    // A record from before migration 18 carries only `ssl`; the backend
    // resolves it to exactly `prefer` / `disable`.
    tlsMode: options?.mode ?? (connection.ssl ? "prefer" : "disable"),
    tlsRootCertPath: options?.rootCertPath ?? "",
    tlsClientCertPath: options?.clientCertPath ?? "",
    tlsClientKeyPath: options?.clientKeyPath ?? "",
    tlsServerName: options?.serverName ?? "",
  };
}

export const FIELD_ERROR = (
  errors: Array<{ message?: string } | string | undefined> | undefined,
): string | null => {
  const first = errors?.[0];
  if (!first) return null;
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof first === "string") return first;
  return first.message ?? null;
};

export type Mode = "new" | "edit";

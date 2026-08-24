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
  // Carried in form state but deliberately not rendered — sqlx 0.8
  // exposes no socket-keepalive setter, so there is no control for a
  // knob nothing applies (ADR-0013 §Decision). Keeping it in the
  // schema is what makes an existing value survive a round-trip
  // instead of being wiped on save.
  keepaliveSeconds: z.number().int().optional(),
  defaultSearchPath: z.string().optional(),
  defaultRole: z.string().optional(),
});

export type ConnectionFormData = z.infer<typeof connectionSchema>;

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
  return {
    ...common,
    engine: "PostgreSQL",
    ssl: value.ssl ?? true,
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
  switch (connection.engine) {
    case "PostgreSQL":
    case "MySQL":
      return {
        ...common,
        ...tunnelDefaults,
        ...driverDefaults,
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

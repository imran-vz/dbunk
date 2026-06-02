/**
 * Pure helpers for `ConnectionForm`: schema, builders, defaults. Kept
 * away from the React tree so they're unit-testable per engine branch
 * without spinning up a form. The per-engine projection lives in
 * `buildStoredConnectionFromForm`, split across one helper per variant
 * to keep each branch trivial (no nested switch + no shared common
 * spread interleaved with per-engine fields).
 */

import * as z from "zod";

import type {
  ClickHouseStoredConnection,
  Connection,
  MySqlStoredConnection,
  PgStoredConnection,
  RedisStoredConnection,
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
};

type CommonShape = {
  id: string;
  name: string;
  database: string;
  host: string;
  port: number;
  user: string;
  password: string;
  role: string;
};

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
  };
}

function buildPg(
  value: ConnectionFormData,
  common: CommonShape,
): PgStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  return {
    ...common,
    engine: "PostgreSQL",
    ssl: value.ssl ?? true,
    ...(sshTunnel ? { sshTunnel } : null),
  };
}

function buildMySql(
  value: ConnectionFormData,
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

function buildSqlite(common: CommonShape): SqliteStoredConnection {
  return { ...common, engine: "SQLite" };
}

function buildClickHouse(
  value: ConnectionFormData,
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
  common: CommonShape,
): RedisStoredConnection {
  const sshTunnel = tunnelFromForm(value);
  return {
    ...common,
    engine: "Redis",
    dbNumber: value.dbNumber ?? 0,
    useTls: value.useTls ?? false,
    verifyTlsCert: value.verifyTlsCert ?? true,
    readOnly: value.readOnly ?? false,
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
  switch (connection.engine) {
    case "PostgreSQL":
    case "MySQL":
      return {
        ...common,
        ...tunnelDefaults,
        ssl: connection.ssl,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: false,
      };
    case "SQLite":
      return {
        ...common,
        ...tunnelDefaults,
        ssl: true,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: false,
      };
    case "ClickHouse":
      return {
        ...common,
        ...tunnelDefaults,
        ssl: true,
        useHttps: connection.useHttps,
        urlPath: connection.urlPath,
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
        readOnly: false,
      };
    case "Redis":
      return {
        ...common,
        ...tunnelDefaults,
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

export const FIELD_ERROR = (
  errors: Array<{ message?: string } | string | undefined> | undefined,
): string | null => {
  const first = errors?.[0];
  if (!first) return null;
  if (typeof first === "string") return first;
  return first.message ?? null;
};

export type Mode = "new" | "edit";

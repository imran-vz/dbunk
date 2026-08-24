/**
 * Secret-free connection URI building and parsing (Plan 009, PAR-005).
 *
 * Build: `Copy URI` emits `engine://user@host:port/database` and NEVER
 * includes a password — there is deliberately no include-secret option.
 * SQLite has no canonical URI and ClickHouse's HTTP endpoint shape is
 * ambiguous, so both refuse with a reason the UI can act on (hide the
 * affordance).
 *
 * Parse: `Import from URI` accepts `postgres://`, `postgresql://`,
 * `mysql://`, `redis://`, and `rediss://`. Query parameters are ignored
 * in v1 and reported back via `ignoredParams` so the form can say so.
 */

import type { Connection } from "@/lib/store";

export type BuildUriResult =
  | { ok: true; uri: string }
  | { ok: false; reason: string };

export type ParsedConnectionUri = {
  engine: "PostgreSQL" | "MySQL" | "Redis";
  host: string;
  port: number;
  user: string;
  /** Present only when the URI carried one; imported, never re-emitted. */
  password?: string;
  database: string;
  /** Redis only: numeric path segment (`redis://host/2`). */
  dbNumber?: number;
  /** Redis only: true for `rediss://`. */
  useTls?: boolean;
  /** Query parameters we understood syntactically but do not apply. */
  ignoredParams: string[];
};

export type ParseUriResult =
  | { ok: true; values: ParsedConnectionUri }
  | { ok: false; reason: string };

const DEFAULT_PORTS = {
  PostgreSQL: 5432,
  MySQL: 3306,
  Redis: 6379,
} as const;

const SCHEME_TO_ENGINE = new Map<
  string,
  { engine: ParsedConnectionUri["engine"]; useTls?: boolean }
>([
  ["postgres", { engine: "PostgreSQL" }],
  ["postgresql", { engine: "PostgreSQL" }],
  ["mysql", { engine: "MySQL" }],
  ["redis", { engine: "Redis", useTls: false }],
  ["rediss", { engine: "Redis", useTls: true }],
]);

/** RFC 3986 userinfo/segment encoding via the strictest builtin. */
const encodeSegment = (value: string) => encodeURIComponent(value);

/** The subset of a `Connection` the builder needs; `useTls`/`dbNumber`
 * only exist on the Redis variant, hence the standalone shape. */
export type UriConnectionInput = {
  engine: Connection["engine"];
  user: string;
  host: string;
  port: number;
  database: string;
  useTls?: boolean;
  dbNumber?: number;
};

export function buildConnectionUri(
  connection: UriConnectionInput,
): BuildUriResult {
  switch (connection.engine) {
    case "SQLite":
      return {
        ok: false,
        reason: "SQLite connections are file paths and have no canonical URI.",
      };
    case "ClickHouse":
      return {
        ok: false,
        reason:
          "ClickHouse connections use an HTTP endpoint with no single canonical URI shape.",
      };
    default:
      break;
  }

  const scheme =
    connection.engine === "PostgreSQL"
      ? "postgres"
      : connection.engine === "MySQL"
        ? "mysql"
        : connection.useTls
          ? "rediss"
          : "redis";
  const userInfo = connection.user
    ? `${encodeSegment(connection.user)}@`
    : "";
  const rawHost = connection.host || "localhost";
  // Bracket bare IPv6 literals so `host:port` stays parseable.
  const host =
    rawHost.includes(":") && !rawHost.startsWith("[")
      ? `[${rawHost}]`
      : rawHost;
  const port = connection.port || DEFAULT_PORTS[connection.engine];
  const path =
    connection.engine === "Redis"
      ? connection.dbNumber
        ? `/${connection.dbNumber}`
        : ""
      : connection.database
        ? `/${encodeSegment(connection.database)}`
        : "";
  return { ok: true, uri: `${scheme}://${userInfo}${host}:${port}${path}` };
}

export function parseConnectionUri(input: string): ParseUriResult {
  const trimmed = input.trim();
  if (!trimmed) {
    return { ok: false, reason: "Paste a connection URI first." };
  }
  const schemeMatch = /^([a-zA-Z][a-zA-Z0-9+.-]*):\/\//.exec(trimmed);
  const accepted =
    "Accepted schemes: postgres://, postgresql://, mysql://, redis://, rediss://.";
  if (!schemeMatch) {
    return { ok: false, reason: `Not a connection URI. ${accepted}` };
  }
  const mapping = SCHEME_TO_ENGINE.get(schemeMatch[1].toLowerCase());
  if (!mapping) {
    return {
      ok: false,
      reason: `Unsupported scheme "${schemeMatch[1]}". ${accepted}`,
    };
  }

  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return { ok: false, reason: `Malformed URI. ${accepted}` };
  }

  const decode = (value: string) => {
    try {
      return decodeURIComponent(value);
    } catch {
      return value;
    }
  };

  const firstSegment = decode(url.pathname.replace(/^\//, "").split("/")[0]);
  const port = url.port
    ? Number.parseInt(url.port, 10)
    : DEFAULT_PORTS[mapping.engine];

  // WHATWG URL keeps IPv6 brackets on `hostname`; the connection form
  // and backend expect the bare literal.
  const hostname = url.hostname.replace(/^\[(.*)\]$/, "$1");

  const values: ParsedConnectionUri = {
    engine: mapping.engine,
    host: hostname,
    port,
    user: decode(url.username),
    ...(url.password ? { password: decode(url.password) } : null),
    database: mapping.engine === "Redis" ? "" : firstSegment,
    ignoredParams: [...new Set(url.searchParams.keys())],
  };
  if (mapping.engine === "Redis") {
    values.useTls = mapping.useTls ?? false;
    const dbNumber = Number.parseInt(firstSegment, 10);
    if (Number.isInteger(dbNumber) && dbNumber >= 0 && dbNumber <= 15) {
      values.dbNumber = dbNumber;
    }
  }
  return { ok: true, values };
}

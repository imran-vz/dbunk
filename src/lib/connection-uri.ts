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
 * `mysql://`, `redis://`, and `rediss://`. The only query parameter
 * applied is a valid PostgreSQL `sslmode` (ADR-0025); every other one —
 * including `sslrootcert` / `sslcert` / `sslkey`, because paths from a
 * pasted URI are not trusted blindly — is reported via `ignoredParams`
 * so the form can say so.
 */

import { connectionFormPolicy } from "@/lib/engine-policy";
import type { Connection } from "@/lib/store";
import { PG_TLS_MODES, type PgTlsMode } from "@/lib/store/types";

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
  /** PostgreSQL only: a valid `sslmode` query parameter. */
  tlsMode?: PgTlsMode;
  /** Query parameters we understood syntactically but do not apply. */
  ignoredParams: string[];
  /** Parts of the URI that were dropped for another reason (e.g. a
   *  Redis db number outside the form's range) — disclosed, never
   *  silently lost. */
  warnings: string[];
};

export type ParseUriResult =
  | { ok: true; values: ParsedConnectionUri }
  | { ok: false; reason: string };

/** Per-engine port defaults come from engine policy — the single
 *  authority the connection form already reads. */
const defaultPortFor = (engine: ParsedConnectionUri["engine"]): number => {
  const form = connectionFormPolicy(engine);
  switch (form.kind) {
    case "host-auth":
    case "redis":
      return form.defaultPort;
    case "clickhouse-http":
      return form.defaultPortHttp;
    case "file":
      return 0;
  }
};

const redisMaxDbNumber = (): number => {
  const form = connectionFormPolicy("Redis");
  return form.kind === "redis" ? form.maxDbNumber : 15;
};

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
  /** PostgreSQL legacy TLS flag; `tlsOptions.mode` wins when present. */
  ssl?: boolean;
  tlsOptions?: { mode: PgTlsMode };
};

/** The mode a PostgreSQL record resolves to — the backend's own rule. */
const resolvedPgTlsMode = (connection: UriConnectionInput): PgTlsMode =>
  connection.tlsOptions?.mode ??
  (connection.ssl === false ? "disable" : "prefer");

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
  const userInfo = connection.user ? `${encodeSegment(connection.user)}@` : "";
  const rawHost = connection.host || "localhost";
  // Bracket bare IPv6 literals so `host:port` stays parseable.
  const host =
    rawHost.includes(":") && !rawHost.startsWith("[")
      ? `[${rawHost}]`
      : rawHost;
  const port = connection.port || defaultPortFor(connection.engine);
  const path =
    connection.engine === "Redis"
      ? connection.dbNumber
        ? `/${connection.dbNumber}`
        : ""
      : connection.database
        ? `/${encodeSegment(connection.database)}`
        : "";
  // `prefer` is libpq's default, so only a departure from it is worth
  // carrying; `sslmode` is the one parameter the parser applies.
  const tlsMode =
    connection.engine === "PostgreSQL" ? resolvedPgTlsMode(connection) : null;
  const query = tlsMode && tlsMode !== "prefer" ? `?sslmode=${tlsMode}` : "";
  return {
    ok: true,
    uri: `${scheme}://${userInfo}${host}:${port}${path}${query}`,
  };
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
    : defaultPortFor(mapping.engine);

  // WHATWG URL keeps IPv6 brackets on `hostname`; the connection form
  // and backend expect the bare literal.
  const hostname = url.hostname.replace(/^\[(.*)\]$/, "$1");

  const sslMode = url.searchParams.get("sslmode");
  const tlsMode =
    mapping.engine === "PostgreSQL"
      ? PG_TLS_MODES.find((mode) => mode === sslMode)
      : undefined;
  const values: ParsedConnectionUri = {
    engine: mapping.engine,
    host: hostname,
    port,
    user: decode(url.username),
    ...(url.password ? { password: decode(url.password) } : null),
    database: mapping.engine === "Redis" ? "" : firstSegment,
    ...(tlsMode ? { tlsMode } : null),
    // An invalid `sslmode` stays here: reported, never guessed at.
    ignoredParams: [...new Set(url.searchParams.keys())].filter(
      (key) => !(tlsMode && key === "sslmode"),
    ),
    warnings: [],
  };
  if (mapping.engine === "Redis") {
    values.useTls = mapping.useTls ?? false;
    const maxDbNumber = redisMaxDbNumber();
    const dbNumber = /^\d+$/.test(firstSegment)
      ? Number.parseInt(firstSegment, 10)
      : Number.NaN;
    if (
      Number.isInteger(dbNumber) &&
      dbNumber >= 0 &&
      dbNumber <= maxDbNumber
    ) {
      values.dbNumber = dbNumber;
    } else if (firstSegment) {
      values.warnings.push(
        `Database "${firstSegment}" was not applied — the DB number must be 0–${maxDbNumber}.`,
      );
    }
  }
  return { ok: true, values };
}

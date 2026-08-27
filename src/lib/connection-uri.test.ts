import { describe, expect, it } from "vitest";

import { buildConnectionUri, parseConnectionUri } from "./connection-uri";

describe("buildConnectionUri", () => {
  it("builds a postgres URI without any password", () => {
    const result = buildConnectionUri({
      engine: "PostgreSQL",
      user: "app_user",
      host: "db.internal",
      port: 5433,
      database: "orders",
    });
    expect(result).toEqual({
      ok: true,
      uri: "postgres://app_user@db.internal:5433/orders",
    });
  });

  it("percent-encodes user and database segments", () => {
    const result = buildConnectionUri({
      engine: "MySQL",
      user: "user name",
      host: "localhost",
      port: 3306,
      database: "sales db",
    });
    expect(result).toEqual({
      ok: true,
      uri: "mysql://user%20name@localhost:3306/sales%20db",
    });
  });

  it("falls back to the engine default port and localhost", () => {
    const result = buildConnectionUri({
      engine: "PostgreSQL",
      user: "",
      host: "",
      port: 0,
      database: "",
    });
    expect(result).toEqual({ ok: true, uri: "postgres://localhost:5432" });
  });

  it("uses rediss and the db-number path for TLS Redis", () => {
    const result = buildConnectionUri({
      engine: "Redis",
      user: "",
      host: "cache.internal",
      port: 6380,
      database: "",
      useTls: true,
      dbNumber: 3,
    });
    expect(result).toEqual({
      ok: true,
      uri: "rediss://cache.internal:6380/3",
    });
  });

  it("refuses SQLite and ClickHouse with a reason", () => {
    for (const engine of ["SQLite", "ClickHouse"] as const) {
      const result = buildConnectionUri({
        engine,
        user: "u",
        host: "h",
        port: 1,
        database: "d",
      });
      expect(result.ok).toBe(false);
      if (!result.ok) {
        expect(result.reason.length).toBeGreaterThan(0);
      }
    }
  });
});

describe("parseConnectionUri", () => {
  it("round-trips what buildConnectionUri emits", () => {
    const built = buildConnectionUri({
      engine: "PostgreSQL",
      user: "app user",
      host: "db.internal",
      port: 5433,
      database: "orders db",
    });
    expect(built.ok).toBe(true);
    if (!built.ok) return;
    const parsed = parseConnectionUri(built.uri);
    expect(parsed).toEqual({
      ok: true,
      values: {
        engine: "PostgreSQL",
        host: "db.internal",
        port: 5433,
        user: "app user",
        database: "orders db",
        ignoredParams: [],
        warnings: [],
      },
    });
  });

  it("captures a password on parse", () => {
    const parsed = parseConnectionUri(
      "postgresql://admin:s%40cret@10.0.0.1/app",
    );
    expect(parsed).toEqual({
      ok: true,
      values: {
        engine: "PostgreSQL",
        host: "10.0.0.1",
        port: 5432,
        user: "admin",
        password: "s@cret",
        database: "app",
        ignoredParams: [],
        warnings: [],
      },
    });
  });

  it("applies engine default ports when the URI has none", () => {
    const mysql = parseConnectionUri("mysql://root@db/main");
    expect(mysql.ok && mysql.values.port).toBe(3306);
    const redis = parseConnectionUri("redis://cache");
    expect(redis.ok && redis.values.port).toBe(6379);
  });

  it("maps rediss to TLS Redis and a numeric path to dbNumber", () => {
    const parsed = parseConnectionUri("rediss://cache.internal:6380/2");
    expect(parsed).toEqual({
      ok: true,
      values: {
        engine: "Redis",
        host: "cache.internal",
        port: 6380,
        user: "",
        database: "",
        useTls: true,
        dbNumber: 2,
        ignoredParams: [],
        warnings: [],
      },
    });
  });

  it("discloses a Redis db number outside the form range instead of dropping it", () => {
    const parsed = parseConnectionUri("redis://cache.internal/20");
    expect(parsed.ok && parsed.values.dbNumber).toBeUndefined();
    expect(parsed.ok && parsed.values.warnings).toEqual([
      'Database "20" was not applied — the DB number must be 0–15.',
    ]);
    const junk = parseConnectionUri("redis://cache.internal/abc");
    expect(junk.ok && junk.values.warnings).toHaveLength(1);
  });

  it("reports ignored query parameters without applying them", () => {
    const parsed = parseConnectionUri(
      "postgres://u@h:5432/db?sslmode=require&connect_timeout=5",
    );
    // `sslmode` is the one parameter applied (ADR-0025); the rest are
    // still disclosed.
    expect(parsed.ok && parsed.values).toMatchObject({
      tlsMode: "require",
      ignoredParams: ["connect_timeout"],
    });
  });

  it("round-trips IPv6 hosts through brackets", () => {
    const built = buildConnectionUri({
      engine: "PostgreSQL",
      user: "app",
      host: "::1",
      port: 5432,
      database: "orders",
    });
    expect(built).toEqual({
      ok: true,
      uri: "postgres://app@[::1]:5432/orders",
    });
    if (!built.ok) return;
    const parsed = parseConnectionUri(built.uri);
    expect(parsed.ok && parsed.values.host).toBe("::1");
    expect(parsed.ok && parsed.values.port).toBe(5432);
  });

  it("refuses unsupported schemes, junk, and empty input", () => {
    for (const input of [
      "mongodb://h/db",
      "not a uri",
      "http://example.com",
      "",
      "   ",
    ]) {
      const parsed = parseConnectionUri(input);
      expect(parsed.ok).toBe(false);
      if (!parsed.ok) {
        expect(parsed.reason.length).toBeGreaterThan(0);
      }
    }
  });
});

describe("PostgreSQL sslmode round-trip (ADR-0025)", () => {
  const pg = {
    engine: "PostgreSQL" as const,
    user: "app",
    host: "db.example.com",
    port: 5432,
    database: "orders",
  };

  it("emits sslmode only when the resolved mode is not prefer", () => {
    expect(buildConnectionUri(pg)).toEqual({
      ok: true,
      uri: "postgres://app@db.example.com:5432/orders",
    });
    expect(buildConnectionUri({ ...pg, ssl: true })).toEqual({
      ok: true,
      uri: "postgres://app@db.example.com:5432/orders",
    });
    expect(
      buildConnectionUri({ ...pg, tlsOptions: { mode: "prefer" } }),
    ).toEqual({ ok: true, uri: "postgres://app@db.example.com:5432/orders" });
    expect(
      buildConnectionUri({ ...pg, tlsOptions: { mode: "verify-full" } }),
    ).toEqual({
      ok: true,
      uri: "postgres://app@db.example.com:5432/orders?sslmode=verify-full",
    });
  });

  it("resolves a legacy ssl:false record to disable", () => {
    expect(buildConnectionUri({ ...pg, ssl: false })).toEqual({
      ok: true,
      uri: "postgres://app@db.example.com:5432/orders?sslmode=disable",
    });
  });

  it("parses a valid sslmode into tlsMode and stops reporting it as ignored", () => {
    const parsed = parseConnectionUri(
      "postgres://app@db.example.com:5432/orders?sslmode=verify-ca",
    );
    expect(parsed.ok && parsed.values).toMatchObject({
      tlsMode: "verify-ca",
      ignoredParams: [],
    });
  });

  it("round-trips the mode through build → parse", () => {
    const built = buildConnectionUri({
      ...pg,
      tlsOptions: { mode: "require" },
    });
    if (!built.ok) throw new Error(built.reason);
    const parsed = parseConnectionUri(built.uri);
    expect(parsed.ok && parsed.values.tlsMode).toBe("require");
  });

  it("reports an invalid sslmode and certificate paths as ignored", () => {
    const parsed = parseConnectionUri(
      "postgres://app@h/db?sslmode=maybe&sslrootcert=/ca.pem&sslcert=/c.crt&sslkey=/c.key",
    );
    if (!parsed.ok) throw new Error(parsed.reason);
    expect(parsed.values.tlsMode).toBeUndefined();
    expect(parsed.values.ignoredParams).toEqual([
      "sslmode",
      "sslrootcert",
      "sslcert",
      "sslkey",
    ]);
  });

  it("does not apply sslmode to engines without TLS modes", () => {
    const parsed = parseConnectionUri("mysql://app@h:3306/db?sslmode=disable");
    if (!parsed.ok) throw new Error(parsed.reason);
    expect(parsed.values.tlsMode).toBeUndefined();
    expect(parsed.values.ignoredParams).toEqual(["sslmode"]);
  });
});

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
      },
    });
  });

  it("reports ignored query parameters without applying them", () => {
    const parsed = parseConnectionUri(
      "postgres://u@h:5432/db?sslmode=require&connect_timeout=5",
    );
    expect(parsed.ok && parsed.values.ignoredParams).toEqual([
      "sslmode",
      "connect_timeout",
    ]);
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

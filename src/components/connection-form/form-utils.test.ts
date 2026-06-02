import { describe, expect, it } from "vitest";

import {
  buildConnectionFromForm,
  buildStoredConnectionFromForm,
  type ConnectionFormData,
  defaultValuesFromConnection,
  EMPTY_NEW_DEFAULTS,
  FIELD_ERROR,
} from "./form-utils";

const baseForm: ConnectionFormData = {
  ...EMPTY_NEW_DEFAULTS,
  name: "Primary",
  host: "db.example.com",
  database: "core",
  port: 5432,
  user: "alice",
  password: "secret",
  role: "read/write",
};

describe("buildStoredConnectionFromForm", () => {
  it("projects PostgreSQL values with ssl default true", () => {
    const out = buildStoredConnectionFromForm(
      { ...baseForm, engine: "PostgreSQL", ssl: undefined },
      "id-pg",
    );
    expect(out).toEqual({
      id: "id-pg",
      name: "Primary",
      database: "core",
      host: "db.example.com",
      port: 5432,
      user: "alice",
      password: "secret",
      role: "read/write",
      engine: "PostgreSQL",
      ssl: true,
    });
  });

  it("projects PostgreSQL with ssl explicitly false", () => {
    const out = buildStoredConnectionFromForm(
      { ...baseForm, engine: "PostgreSQL", ssl: false },
      "id-pg2",
    );
    expect(out).toMatchObject({ engine: "PostgreSQL", ssl: false });
  });

  it("projects MySQL values with ssl default true", () => {
    const out = buildStoredConnectionFromForm(
      { ...baseForm, engine: "MySQL", ssl: undefined, port: 3306 },
      "id-mysql",
    );
    expect(out).toMatchObject({
      id: "id-mysql",
      engine: "MySQL",
      ssl: true,
      port: 3306,
    });
  });

  it("projects SQLite values with file path in database", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "SQLite",
        database: "/tmp/local.db",
        host: "",
        port: 0,
        user: "",
        password: "",
      },
      "id-sqlite",
    );
    expect(out).toEqual({
      id: "id-sqlite",
      name: "Primary",
      database: "/tmp/local.db",
      host: "",
      port: 0,
      user: "",
      password: "",
      role: "read/write",
      engine: "SQLite",
    });
    // No engine-specific fields on SQLite — should never include ssl etc.
    expect(out).not.toHaveProperty("ssl");
    expect(out).not.toHaveProperty("useHttps");
  });

  it("projects ClickHouse with defaults when toggles are undefined", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "ClickHouse",
        useHttps: undefined,
        urlPath: undefined,
        port: 8123,
      },
      "id-ch",
    );
    expect(out).toMatchObject({
      engine: "ClickHouse",
      useHttps: false,
      urlPath: "",
      port: 8123,
    });
  });

  it("projects ClickHouse with HTTPS + custom URL path", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "ClickHouse",
        useHttps: true,
        urlPath: "/clickhouse",
        port: 8443,
      },
      "id-ch2",
    );
    expect(out).toMatchObject({
      engine: "ClickHouse",
      useHttps: true,
      urlPath: "/clickhouse",
    });
  });

  it("projects Redis with all defaults when toggles are undefined", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "Redis",
        dbNumber: undefined,
        useTls: undefined,
        verifyTlsCert: undefined,
        port: 6379,
      },
      "id-redis",
    );
    expect(out).toMatchObject({
      engine: "Redis",
      dbNumber: 0,
      useTls: false,
      verifyTlsCert: true,
    });
  });

  it("projects Redis with TLS off and verifyTlsCert preserved", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "Redis",
        dbNumber: 3,
        useTls: false,
        verifyTlsCert: false,
      },
      "id-redis2",
    );
    expect(out).toMatchObject({
      engine: "Redis",
      dbNumber: 3,
      useTls: false,
      verifyTlsCert: false,
    });
  });

  it("projects Redis with TLS on + verify on", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "Redis",
        dbNumber: 7,
        useTls: true,
        verifyTlsCert: true,
      },
      "id-redis3",
    );
    expect(out).toMatchObject({
      engine: "Redis",
      dbNumber: 7,
      useTls: true,
      verifyTlsCert: true,
    });
  });

  it("substitutes empty role with `read/write` default", () => {
    const out = buildStoredConnectionFromForm(
      { ...baseForm, engine: "PostgreSQL", role: "" },
      "id-role",
    );
    expect(out.role).toBe("read/write");
  });

  it("preserves a non-empty role verbatim", () => {
    const out = buildStoredConnectionFromForm(
      { ...baseForm, engine: "PostgreSQL", role: "read-only" },
      "id-role2",
    );
    expect(out.role).toBe("read-only");
  });

  it("substitutes empty strings/zero for undefined common fields", () => {
    const out = buildStoredConnectionFromForm(
      {
        engine: "PostgreSQL",
        name: "Just a name",
        host: undefined,
        port: undefined,
        user: undefined,
        password: undefined,
        database: undefined,
        role: undefined,
      } as ConnectionFormData,
      "id-defaults",
    );
    expect(out).toMatchObject({
      host: "",
      port: 0,
      user: "",
      password: "",
      database: "",
      role: "read/write",
    });
  });

  it("projects SSH tunnel config only for network-backed engines", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "PostgreSQL",
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
        sshTunnelLocalBindHost: " 127.0.0.1 ",
        sshTunnelLocalPort: 15432,
      },
      "id-tunnel",
    );
    expect(out).toMatchObject({
      engine: "PostgreSQL",
      sshTunnel: {
        enabled: true,
        bastionServerId: "bastion-1",
        localBindHost: "127.0.0.1",
        localPort: 15432,
      },
    });
  });

  it("omits SSH tunnel config when disabled", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "Redis",
        sshTunnelEnabled: false,
        sshTunnelBastionServerId: "bastion-1",
      },
      "id-no-tunnel",
    );
    expect(out).not.toHaveProperty("sshTunnel");
  });

  it("excludes SSH tunnel config from SQLite", () => {
    const out = buildStoredConnectionFromForm(
      {
        ...baseForm,
        engine: "SQLite",
        database: "/tmp/local.db",
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
      },
      "id-sqlite-tunnel",
    );
    expect(out).not.toHaveProperty("sshTunnel");
  });
});

describe("buildConnectionFromForm", () => {
  it("merges runtime fields onto the stored projection", () => {
    const out = buildConnectionFromForm(
      { ...baseForm, engine: "PostgreSQL" },
      "id-runtime",
      {
        status: "Connected",
        latency: "12ms",
        errorMessage: undefined,
        lastActivityAt: "2024-01-02T00:00:00Z",
      },
    );
    expect(out).toMatchObject({
      id: "id-runtime",
      engine: "PostgreSQL",
      status: "Connected",
      latency: "12ms",
      lastActivityAt: "2024-01-02T00:00:00Z",
    });
  });
});

describe("defaultValuesFromConnection", () => {
  it("hydrates Redis fields and blanks the password", () => {
    const values = defaultValuesFromConnection({
      id: "x",
      name: "rdb",
      engine: "Redis",
      host: "r.example.com",
      database: "",
      port: 6379,
      user: "default",
      password: "should-not-appear",
      role: "read/write",
      dbNumber: 2,
      useTls: true,
      verifyTlsCert: false,
      readOnly: false,
      status: "Connected",
      latency: "1ms",
    });
    expect(values.password).toBe("");
    expect(values).toMatchObject({
      engine: "Redis",
      dbNumber: 2,
      useTls: true,
      verifyTlsCert: false,
    });
  });

  it("hydrates SSH tunnel defaults from a stored network connection", () => {
    const values = defaultValuesFromConnection({
      id: "x",
      name: "pg",
      engine: "PostgreSQL",
      host: "pg.internal",
      database: "postgres",
      port: 5432,
      user: "postgres",
      password: "should-not-appear",
      role: "read/write",
      ssl: true,
      sshTunnel: {
        enabled: true,
        bastionServerId: "bastion-1",
        localBindHost: "127.0.0.2",
        localPort: 15432,
      },
      status: "Disconnected",
      latency: "--",
    });
    expect(values.password).toBe("");
    expect(values).toMatchObject({
      sshTunnelEnabled: true,
      sshTunnelBastionServerId: "bastion-1",
      sshTunnelLocalBindHost: "127.0.0.2",
      sshTunnelLocalPort: 15432,
    });
  });
});

describe("FIELD_ERROR", () => {
  it("returns null when there are no errors", () => {
    expect(FIELD_ERROR(undefined)).toBeNull();
    expect(FIELD_ERROR([])).toBeNull();
    expect(FIELD_ERROR([undefined])).toBeNull();
  });

  it("unwraps a string error", () => {
    expect(FIELD_ERROR(["oops"])).toBe("oops");
  });

  it("unwraps an object error's message", () => {
    expect(FIELD_ERROR([{ message: "bad" }])).toBe("bad");
  });

  it("returns null when the object error has no message", () => {
    expect(FIELD_ERROR([{}])).toBeNull();
  });
});

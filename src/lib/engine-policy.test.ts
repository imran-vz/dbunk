import { describe, expect, it } from "vitest";

import type { DatabaseEngine } from "@/lib/store";

import {
  type ConnectionFormValues,
  connectionFormPolicy,
  enginePolicy,
  keyvaluePolicy,
  relationalPolicy,
  storageClassFor,
  validateConnection,
} from "./engine-policy";

const baseValues = (
  overrides: Partial<ConnectionFormValues> = {},
): ConnectionFormValues => ({
  name: "My DB",
  engine: "PostgreSQL",
  host: "localhost",
  database: "core",
  port: 5432,
  user: "postgres",
  password: "hunter2",
  ssl: true,
  ...overrides,
});

const pathsOf = (issues: ReturnType<typeof validateConnection>) =>
  issues.map((issue) => issue.path);

/**
 * Every test here is a small assertion against the static policy
 * table — there's no logic to exercise beyond "the right cell has
 * the right value." We test the shape of the contract:
 *
 *   - exhaustiveness over `DatabaseEngine`
 *   - the per-engine knobs that real components read
 *   - storage-class fork (ADR-0008): Redis carries keyvalue-shaped
 *     fields, relational engines carry relational-shaped fields
 *   - narrowing helpers reject the wrong class
 *
 * The Rust mirror (`src-tauri/src/dispatch.rs::storage_class_is_stable_per_engine`)
 * asserts the same `engine → storage class` classification; drift
 * between the two breaks CI.
 */

const ALL_ENGINES: DatabaseEngine[] = [
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
  "Redis",
];

const RELATIONAL_ENGINES = [
  "PostgreSQL",
  "MySQL",
  "SQLite",
  "ClickHouse",
] as const;

describe("enginePolicy", () => {
  it("returns a policy for every engine variant", () => {
    for (const engine of ALL_ENGINES) {
      const policy = enginePolicy(engine);
      expect(policy.engine).toBe(engine);
    }
  });

  it("classifies relational vs keyvalue engines correctly", () => {
    for (const engine of RELATIONAL_ENGINES) {
      expect(storageClassFor(engine)).toBe("relational");
    }
    expect(storageClassFor("Redis")).toBe("keyvalue");
  });

  it("identifies ClickHouse as the only relational engine without foreign keys", () => {
    expect(relationalPolicy("PostgreSQL").hasForeignKeys).toBe(true);
    expect(relationalPolicy("MySQL").hasForeignKeys).toBe(true);
    expect(relationalPolicy("SQLite").hasForeignKeys).toBe(true);
    expect(relationalPolicy("ClickHouse").hasForeignKeys).toBe(false);
  });

  it("renders ClickHouse FK-unsupported copy as an engine fact, not a coverage gap", () => {
    expect(relationalPolicy("ClickHouse").foreignKeysUnsupportedCopy).toBe(
      "ClickHouse does not support foreign keys.",
    );
    expect(relationalPolicy("PostgreSQL").foreignKeysUnsupportedCopy).toContain(
      "this engine",
    );
  });

  it("uses 'Sorting key' / 'ORDER BY' for ClickHouse, 'Primary key' / 'PK' for other relational engines", () => {
    expect(relationalPolicy("ClickHouse").labels.primaryKey).toBe(
      "Sorting key",
    );
    expect(relationalPolicy("ClickHouse").labels.primaryKeyBadge).toBe(
      "ORDER BY",
    );
    for (const engine of ["PostgreSQL", "MySQL", "SQLite"] as const) {
      expect(relationalPolicy(engine).labels.primaryKey).toBe("Primary key");
      expect(relationalPolicy(engine).labels.primaryKeyBadge).toBe("PK");
    }
  });

  it("uses 'Skip indices' for ClickHouse, 'Indexes' elsewhere", () => {
    expect(relationalPolicy("ClickHouse").labels.indexes).toBe("Skip indices");
    expect(relationalPolicy("PostgreSQL").labels.indexes).toBe("Indexes");
  });

  it("reports ClickHouse row counts as exact, others as estimates", () => {
    expect(relationalPolicy("ClickHouse").rowCountKind).toBe("exact");
    expect(relationalPolicy("PostgreSQL").rowCountKind).toBe("estimate");
    expect(relationalPolicy("MySQL").rowCountKind).toBe("estimate");
    expect(relationalPolicy("SQLite").rowCountKind).toBe("estimate");
  });

  it("exposes a non-null schema-map FK banner only for ClickHouse", () => {
    expect(relationalPolicy("ClickHouse").schemaMapNoForeignKeysCopy).toMatch(
      /ClickHouse/,
    );
    expect(
      relationalPolicy("PostgreSQL").schemaMapNoForeignKeysCopy,
    ).toBeNull();
    expect(relationalPolicy("MySQL").schemaMapNoForeignKeysCopy).toBeNull();
    expect(relationalPolicy("SQLite").schemaMapNoForeignKeysCopy).toBeNull();
  });

  it("groups engines by connection-form shape via the kind discriminator", () => {
    expect(enginePolicy("SQLite").connectionForm.kind).toBe("file");
    expect(enginePolicy("PostgreSQL").connectionForm.kind).toBe("host-auth");
    expect(enginePolicy("MySQL").connectionForm.kind).toBe("host-auth");
    expect(enginePolicy("ClickHouse").connectionForm.kind).toBe(
      "clickhouse-http",
    );
    expect(enginePolicy("Redis").connectionForm.kind).toBe("redis");
  });

  it("renders PostgreSQL TLS modes and the MySQL toggle on the shared host-auth kind", () => {
    const pg = enginePolicy("PostgreSQL").connectionForm;
    const my = enginePolicy("MySQL").connectionForm;
    if (pg.kind !== "host-auth" || my.kind !== "host-auth") {
      throw new Error("expected host-auth kind for PG/MySQL");
    }
    expect(pg.tlsControls).toBe("postgres-modes");
    expect(my.tlsControls).toBe("toggle");
  });

  it("surfaces driver options on PG only, not on the shared host-auth kind", () => {
    const pg = enginePolicy("PostgreSQL").connectionForm;
    const my = enginePolicy("MySQL").connectionForm;
    if (pg.kind !== "host-auth" || my.kind !== "host-auth") {
      throw new Error("expected host-auth kind for PG/MySQL");
    }
    expect(pg.showDriverOptions).toBe(true);
    expect(my.showDriverOptions).toBe(false);
  });

  it("knows the canonical default port per engine form-shape", () => {
    const pg = enginePolicy("PostgreSQL").connectionForm;
    const my = enginePolicy("MySQL").connectionForm;
    const ch = enginePolicy("ClickHouse").connectionForm;
    const redis = enginePolicy("Redis").connectionForm;
    if (pg.kind !== "host-auth" || my.kind !== "host-auth") {
      throw new Error("expected host-auth kind for PG/MySQL");
    }
    if (ch.kind !== "clickhouse-http") {
      throw new Error("expected clickhouse-http kind");
    }
    if (redis.kind !== "redis") {
      throw new Error("expected redis kind");
    }
    expect(pg.defaultPort).toBe(5432);
    expect(my.defaultPort).toBe(3306);
    expect(ch.defaultPortHttp).toBe(8123);
    expect(ch.defaultPortHttps).toBe(8443);
    expect(redis.defaultPort).toBe(6379);
  });

  it("returns Redis-shaped keyvalue policy with destructive-command lists", () => {
    const redis = keyvaluePolicy("Redis");
    expect(redis.storageClass).toBe("keyvalue");
    expect(redis.defaultDbNumber).toBe(0);
    expect(redis.maxDbNumber).toBe(15);
    expect(redis.defaultSeparator).toBe(":");
    expect(redis.pubSubSupported).toBe(true);
    expect(redis.transactionsSupported).toBe(true);
    expect(redis.destructiveCommandsHard).toContain("FLUSHDB");
    expect(redis.destructiveCommandsHard).toContain("CONFIG SET");
    expect(redis.destructiveCommandsSoft).toContain("KEYS");
  });

  it("relationalPolicy throws when called with a keyvalue engine", () => {
    expect(() => relationalPolicy("Redis")).toThrow(/keyvalue/);
  });

  it("keyvaluePolicy throws when called with a relational engine", () => {
    expect(() => keyvaluePolicy("PostgreSQL")).toThrow(/relational/);
  });
});

describe("validateConnection", () => {
  it("returns no issues for a fully-populated host-auth value (new mode)", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    expect(validateConnection(policy, baseValues(), "new")).toEqual([]);
  });

  it("requires name across every kind", () => {
    for (const engine of [
      "PostgreSQL",
      "MySQL",
      "SQLite",
      "ClickHouse",
      "Redis",
    ] as const) {
      const policy = connectionFormPolicy(engine);
      const values =
        engine === "SQLite"
          ? baseValues({ engine, name: "", database: "/tmp/db.sqlite" })
          : baseValues({ engine, name: "" });
      expect(pathsOf(validateConnection(policy, values, "new"))).toContain(
        "name",
      );
    }
  });

  it("requires host + port for host-auth, clickhouse-http, and redis", () => {
    for (const engine of [
      "PostgreSQL",
      "MySQL",
      "ClickHouse",
      "Redis",
    ] as const) {
      const policy = connectionFormPolicy(engine);
      const values = baseValues({
        engine,
        host: "",
        port: 0,
        // Redis password is optional, but the host/port rules still apply.
        password: engine === "Redis" ? undefined : "hunter2",
      });
      const paths = pathsOf(validateConnection(policy, values, "new"));
      expect(paths).toContain("host");
      expect(paths).toContain("port");
    }
  });

  it("requires database + user + password for host-auth (new mode)", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({ database: "", user: "", password: "" }),
      "new",
    );
    expect(pathsOf(issues)).toEqual(
      expect.arrayContaining(["database", "user", "password"]),
    );
  });

  it("relaxes the password rule in edit mode for host-auth", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({ password: "" }),
      "edit",
    );
    expect(pathsOf(issues)).not.toContain("password");
  });

  it("relaxes the password rule in edit mode for clickhouse-http", () => {
    const policy = connectionFormPolicy("ClickHouse");
    const issues = validateConnection(
      policy,
      baseValues({ engine: "ClickHouse", password: "" }),
      "edit",
    );
    expect(pathsOf(issues)).not.toContain("password");
  });

  it("treats user + password as optional for Redis in both modes", () => {
    const policy = connectionFormPolicy("Redis");
    for (const mode of ["new", "edit"] as const) {
      const issues = validateConnection(
        policy,
        baseValues({
          engine: "Redis",
          user: "",
          password: "",
          database: "",
        }),
        mode,
      );
      expect(pathsOf(issues)).not.toContain("user");
      expect(pathsOf(issues)).not.toContain("password");
      expect(pathsOf(issues)).not.toContain("database");
    }
  });

  it("rejects Redis DB numbers outside 0–maxDbNumber", () => {
    const policy = connectionFormPolicy("Redis");
    const tooHigh = validateConnection(
      policy,
      baseValues({ engine: "Redis", dbNumber: 99 }),
      "new",
    );
    const negative = validateConnection(
      policy,
      baseValues({ engine: "Redis", dbNumber: -1 }),
      "new",
    );
    expect(pathsOf(tooHigh)).toContain("dbNumber");
    expect(pathsOf(negative)).toContain("dbNumber");
  });

  it("requires a Bastion Server when SSH tunnel is enabled on network engines", () => {
    for (const engine of [
      "PostgreSQL",
      "MySQL",
      "ClickHouse",
      "Redis",
    ] as const) {
      const policy = connectionFormPolicy(engine);
      const issues = validateConnection(
        policy,
        baseValues({
          engine,
          sshTunnelEnabled: true,
          sshTunnelBastionServerId: "",
          password: engine === "Redis" ? undefined : "hunter2",
        }),
        "new",
      );
      expect(pathsOf(issues)).toContain("sshTunnelBastionServerId");
    }
  });

  it("rejects invalid local SSH tunnel ports", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
        sshTunnelLocalPort: 0,
      }),
      "new",
    );
    expect(pathsOf(issues)).toContain("sshTunnelLocalPort");
  });

  it("rejects invalid SSH keepalive intervals", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
        sshTunnelKeepaliveIntervalSeconds: 1,
      }),
      "new",
    );
    expect(pathsOf(issues)).toContain("sshTunnelKeepaliveIntervalSeconds");
  });

  it("rejects SSH jump chains that loop through the selected Bastion Server", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
        sshTunnelJumpChain: ["jump-1", "bastion-1"],
      }),
      "new",
    );
    expect(pathsOf(issues)).toContain("sshTunnelJumpChain");
  });

  it("rejects duplicate SSH jump-chain Bastion Servers", () => {
    const policy = connectionFormPolicy("PostgreSQL");
    const issues = validateConnection(
      policy,
      baseValues({
        sshTunnelEnabled: true,
        sshTunnelBastionServerId: "bastion-1",
        sshTunnelJumpChain: ["jump-1", " jump-1 "],
      }),
      "new",
    );
    expect(pathsOf(issues)).toContain("sshTunnelJumpChain");
  });

  it("SQLite requires only the database file path", () => {
    const policy = connectionFormPolicy("SQLite");
    const valid = validateConnection(
      policy,
      baseValues({
        engine: "SQLite",
        database: "/tmp/db.sqlite",
        sshTunnelEnabled: true,
      }),
      "new",
    );
    expect(valid).toEqual([]);
    const invalid = validateConnection(
      policy,
      baseValues({ engine: "SQLite", database: "" }),
      "new",
    );
    expect(pathsOf(invalid)).toContain("database");
  });
});

describe("validateConnection — driver options (ADR-0013)", () => {
  const pg = connectionFormPolicy("PostgreSQL");

  it("accepts an entirely blank driver-options block", () => {
    expect(validateConnection(pg, baseValues(), "new")).toEqual([]);
  });

  it("accepts 0 for the session timeouts — PG reads it as no limit", () => {
    const issues = validateConnection(
      pg,
      baseValues({ statementTimeoutMs: 0, idleInTransactionTimeoutMs: 0 }),
      "new",
    );
    expect(issues).toEqual([]);
  });

  it("rejects negative and non-integer session timeouts", () => {
    expect(
      pathsOf(
        validateConnection(pg, baseValues({ statementTimeoutMs: -1 }), "new"),
      ),
    ).toContain("statementTimeoutMs");
    expect(
      pathsOf(
        validateConnection(
          pg,
          baseValues({ idleInTransactionTimeoutMs: 1.5 }),
          "new",
        ),
      ),
    ).toContain("idleInTransactionTimeoutMs");
  });

  it("rejects a session timeout above the 24h unit-mixup guard", () => {
    expect(
      pathsOf(
        validateConnection(
          pg,
          baseValues({ statementTimeoutMs: 86_400_001 }),
          "new",
        ),
      ),
    ).toContain("statementTimeoutMs");
  });

  it("rejects a zero connect timeout — unlike the session timeouts", () => {
    // 0 means "no limit" to PG's SET grammar, but a 0 ms deadline on
    // the handshake wrapper would fail every connect instantly.
    expect(
      pathsOf(
        validateConnection(pg, baseValues({ connectTimeoutMs: 0 }), "new"),
      ),
    ).toContain("connectTimeoutMs");
    expect(
      validateConnection(pg, baseValues({ connectTimeoutMs: 1 }), "new"),
    ).toEqual([]);
    expect(
      pathsOf(
        validateConnection(
          pg,
          baseValues({ connectTimeoutMs: 600_001 }),
          "new",
        ),
      ),
    ).toContain("connectTimeoutMs");
  });

  it("accepts a comma-separated search path and trims around entries", () => {
    expect(
      validateConnection(
        pg,
        baseValues({ defaultSearchPath: " app , public " }),
        "new",
      ),
    ).toEqual([]);
  });

  it("rejects empty search-path entries and embedded double quotes", () => {
    expect(
      pathsOf(
        validateConnection(
          pg,
          baseValues({ defaultSearchPath: "app,,public" }),
          "new",
        ),
      ),
    ).toContain("defaultSearchPath");
    expect(
      pathsOf(
        validateConnection(
          pg,
          baseValues({ defaultSearchPath: 'app"; DROP' }),
          "new",
        ),
      ),
    ).toContain("defaultSearchPath");
  });

  it("ignores driver options on engines whose policy doesn't show them", () => {
    // MySQL shares the host-auth kind but carries no driver_options
    // field — a stale value in shared form state must not block save.
    const my = connectionFormPolicy("MySQL");
    const issues = validateConnection(
      my,
      baseValues({ engine: "MySQL", port: 3306, statementTimeoutMs: -99 }),
      "new",
    );
    expect(pathsOf(issues)).not.toContain("statementTimeoutMs");
  });
});

describe("validateConnection — TLS fields (ADR-0025)", () => {
  const pg = connectionFormPolicy("PostgreSQL");

  it("accepts a client certificate and key together, or neither", () => {
    expect(
      validateConnection(
        pg,
        baseValues({
          tlsMode: "verify-full",
          tlsClientCertPath: "/c.crt",
          tlsClientKeyPath: "/c.key",
        }),
        "new",
      ),
    ).toEqual([]);
    expect(
      validateConnection(pg, baseValues({ tlsMode: "verify-full" }), "new"),
    ).toEqual([]);
  });

  it("rejects a client certificate without its key, and vice versa", () => {
    const certOnly = validateConnection(
      pg,
      baseValues({ tlsMode: "require", tlsClientCertPath: "/c.crt" }),
      "new",
    );
    expect(certOnly).toEqual([
      {
        path: "tlsClientKeyPath",
        message: "Client key path is required with a client certificate",
      },
    ]);
    const keyOnly = validateConnection(
      pg,
      baseValues({ tlsMode: "require", tlsClientKeyPath: "  /c.key " }),
      "new",
    );
    expect(pathsOf(keyOnly)).toEqual(["tlsClientCertPath"]);
  });

  it("ignores stale client-certificate paths once TLS is disabled", () => {
    expect(
      validateConnection(
        pg,
        baseValues({ tlsMode: "disable", tlsClientCertPath: "/c.crt" }),
        "new",
      ),
    ).toEqual([]);
  });

  it("ignores the TLS fields on engines whose policy renders the toggle", () => {
    expect(
      validateConnection(
        connectionFormPolicy("MySQL"),
        baseValues({
          engine: "MySQL",
          port: 3306,
          tlsClientCertPath: "/c.crt",
        }),
        "new",
      ),
    ).toEqual([]);
  });

  it("bounds keepalive idle to 1–7200 whole seconds", () => {
    expect(
      validateConnection(pg, baseValues({ keepaliveSeconds: 60 }), "new"),
    ).toEqual([]);
    for (const keepaliveSeconds of [0, -1, 7201, 1.5]) {
      expect(
        pathsOf(
          validateConnection(pg, baseValues({ keepaliveSeconds }), "new"),
        ),
      ).toEqual(["keepaliveSeconds"]);
    }
  });
});

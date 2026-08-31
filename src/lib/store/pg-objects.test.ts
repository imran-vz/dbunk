/* oxlint-disable anti-slop/no-module-mocking, anti-slop/no-unknown-parameters -- Controlled Tauri boundary fixtures verify generation behavior. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import {
  catalogToSchemaExplorer,
  pgObjectDescriptionKey,
  type PgObjectCatalog,
  type PgObjectDescription,
  useAppStore,
} from "@/lib/store";
import { tauriInvoke } from "@/lib/tauri";

const mockedInvoke = vi.mocked(tauriInvoke);
const initialStoreState = useAppStore.getState();

const catalogFixture = (): PgObjectCatalog => ({
  schemas: [
    {
      name: "lifecycle",
      tables: [{ name: "orders", comment: "Customer orders" }],
      views: [{ name: "orders_view" }],
      materializedViews: [{ name: "orders_mat" }],
      foreignTables: [{ name: "remote_orders" }],
      sequences: [{ name: "order_number_seq" }],
      functions: [
        { name: "add_nums", identityArgs: "integer, integer" },
        { name: "add_nums", identityArgs: "numeric, numeric" },
      ],
      procedures: [{ name: "archive_orders", identityArgs: "date" }],
      aggregates: [{ name: "sum_money", identityArgs: "money" }],
      types: [{ name: "order_status", typeClass: "enum" }],
      domains: [{ name: "positive_amount" }],
      extensions: [{ name: "pgcrypto" }],
    },
  ],
  eventTriggers: [{ name: "audit_ddl" }],
  roles: [{ name: "dbunk" }],
  tablespaces: [{ name: "pg_default" }],
  truncated: [],
});

beforeEach(() => {
  mockedInvoke.mockReset();
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  useAppStore.setState(initialStoreState, true);
});

describe("catalogToSchemaExplorer", () => {
  it("derives every legacy field without a Database pseudo-schema", () => {
    const catalog = catalogFixture();

    expect(catalogToSchemaExplorer(catalog)).toEqual([
      {
        name: "lifecycle",
        tables: ["orders"],
        views: ["orders_view"],
        materializedViews: ["orders_mat"],
        foreignTables: ["remote_orders"],
        sequences: ["order_number_seq"],
        functions: ["add_nums(integer, integer)", "add_nums(numeric, numeric)"],
        procedures: ["archive_orders(date)"],
        aggregateFunctions: ["sum_money(money)"],
        types: ["order_status"],
        domains: ["positive_amount"],
        extensions: ["pgcrypto"],
      },
    ]);
    expect(catalog.schemas[0]?.types[0]?.typeClass).toBe("enum");
  });
});

describe("PgObjectsSlice", () => {
  it("loads one catalog and writes the derived explorer through its owner", async () => {
    const catalog = catalogFixture();
    mockedInvoke.mockResolvedValueOnce(catalog);

    await expect(
      useAppStore.getState().loadPgObjectCatalog("conn-1"),
    ).resolves.toBe("ready");

    expect(mockedInvoke).toHaveBeenCalledOnce();
    expect(mockedInvoke).toHaveBeenCalledWith("load_pg_object_catalog", {
      payload: { connectionId: "conn-1" },
    });
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]).toEqual({
      status: "ready",
      catalog,
      generation: 0,
    });
    expect(useAppStore.getState().schemaExplorer["conn-1"]?.[0]?.name).toBe(
      "lifecycle",
    );
  });

  it("discards a catalog response from a dropped connection generation", async () => {
    const catalog = catalogFixture();
    let resolveCatalog: ((value: PgObjectCatalog) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise<PgObjectCatalog>((resolve) => {
          resolveCatalog = resolve;
        }),
    );

    const load = useAppStore.getState().loadPgObjectCatalog("conn-1");
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]?.status).toBe(
      "loading",
    );

    useAppStore.getState().dropPgObjectCachesForConnection("conn-1");
    resolveCatalog?.(catalog);

    await expect(load).resolves.toBe("stale");
    expect(useAppStore.getState().pgObjectCatalog["conn-1"]).toEqual({
      status: "idle",
      generation: 1,
    });
    expect(useAppStore.getState().schemaExplorer["conn-1"]).toBeUndefined();
  });

  it("keeps the newest same-generation catalog response", async () => {
    const olderCatalog = catalogFixture();
    const newerCatalog: PgObjectCatalog = {
      ...catalogFixture(),
      schemas: catalogFixture().schemas.map((schema) => ({
        ...schema,
        name: "current_schema",
      })),
    };
    let resolveOlder: ((value: PgObjectCatalog) => void) | undefined;
    let resolveNewer: ((value: PgObjectCatalog) => void) | undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise<PgObjectCatalog>((resolve) => {
            resolveOlder = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<PgObjectCatalog>((resolve) => {
            resolveNewer = resolve;
          }),
      );

    const olderLoad = useAppStore.getState().loadPgObjectCatalog("conn-1");
    const newerLoad = useAppStore.getState().loadPgObjectCatalog("conn-1");
    resolveNewer?.(newerCatalog);
    await expect(newerLoad).resolves.toBe("ready");
    resolveOlder?.(olderCatalog);
    await expect(olderLoad).resolves.toBe("stale");

    expect(useAppStore.getState().pgObjectCatalog["conn-1"]?.catalog).toEqual(
      newerCatalog,
    );
    expect(useAppStore.getState().schemaExplorer["conn-1"]?.[0]?.name).toBe(
      "current_schema",
    );
  });

  it("stores a typed visible catalog error", async () => {
    mockedInvoke.mockRejectedValueOnce({
      kind: "database",
      statementIndex: 0,
      code: "42501",
      message: "permission denied for catalog",
      position: null,
      appliedStatements: 0,
    });

    await expect(
      useAppStore.getState().loadPgObjectCatalog("conn-1"),
    ).resolves.toBe("error");

    expect(useAppStore.getState().pgObjectCatalog["conn-1"]?.error).toEqual({
      kind: "database",
      statementIndex: 0,
      code: "42501",
      message: "permission denied for catalog",
      position: null,
      appliedStatements: 0,
    });
  });

  it("discards a description response from a dropped connection generation", async () => {
    const reference = {
      kind: "sequence",
      schema: "lifecycle",
      name: "order_number_seq",
      identityArgs: null,
    } as const;
    const description: PgObjectDescription = {
      reference,
      owner: "dbunk",
      comment: null,
      definitionSql: "CREATE SEQUENCE lifecycle.order_number_seq;",
      facts: {
        kind: "sequence",
        dataType: "bigint",
        start: "1",
        increment: "1",
        minValue: "1",
        maxValue: "9223372036854775807",
        cycle: false,
        cache: "1",
        lastValue: null,
        ownedBy: null,
      },
    };
    let resolveDescription: ((value: PgObjectDescription) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise<PgObjectDescription>((resolve) => {
          resolveDescription = resolve;
        }),
    );

    const load = useAppStore
      .getState()
      .loadPgObjectDescription("conn-1", reference);
    useAppStore.getState().dropPgObjectCachesForConnection("conn-1");
    resolveDescription?.(description);

    await expect(load).resolves.toBe("stale");
    expect(useAppStore.getState().pgObjectDescriptions).toEqual({});
  });

  it("keeps the newest same-generation description response", async () => {
    const reference = {
      kind: "view",
      schema: "lifecycle",
      name: "orders_view",
      identityArgs: null,
    } as const;
    const older: PgObjectDescription = {
      reference,
      owner: "older_owner",
      comment: null,
      definitionSql: "CREATE VIEW lifecycle.orders_view AS SELECT 1;",
      facts: { kind: "view", definition: "SELECT 1" },
    };
    const newer: PgObjectDescription = {
      ...older,
      owner: "current_owner",
      definitionSql: "CREATE VIEW lifecycle.orders_view AS SELECT 2;",
      facts: { kind: "view", definition: "SELECT 2" },
    };
    let resolveOlder: ((value: PgObjectDescription) => void) | undefined;
    let resolveNewer: ((value: PgObjectDescription) => void) | undefined;
    mockedInvoke
      .mockImplementationOnce(
        () =>
          new Promise<PgObjectDescription>((resolve) => {
            resolveOlder = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<PgObjectDescription>((resolve) => {
            resolveNewer = resolve;
          }),
      );

    const olderLoad = useAppStore
      .getState()
      .loadPgObjectDescription("conn-1", reference);
    const newerLoad = useAppStore
      .getState()
      .loadPgObjectDescription("conn-1", reference);
    resolveNewer?.(newer);
    await expect(newerLoad).resolves.toBe("ready");
    resolveOlder?.(older);
    await expect(olderLoad).resolves.toBe("stale");

    const key = pgObjectDescriptionKey("conn-1", reference);
    expect(
      useAppStore.getState().pgObjectDescriptions[key]?.description,
    ).toEqual(newer);
  });

  it("drops a schema and all descendant description caches", () => {
    const schema = {
      kind: "schema",
      schema: null,
      name: "lifecycle",
      identityArgs: null,
    } as const;
    const descendant = {
      kind: "view",
      schema: "lifecycle",
      name: "orders_view",
      identityArgs: null,
    } as const;
    const unrelated = {
      kind: "view",
      schema: "public",
      name: "orders_view",
      identityArgs: null,
    } as const;
    const schemaKey = pgObjectDescriptionKey("conn-1", schema);
    const descendantKey = pgObjectDescriptionKey("conn-1", descendant);
    const unrelatedKey = pgObjectDescriptionKey("conn-1", unrelated);
    useAppStore.setState({
      pgObjectDescriptions: {
        [schemaKey]: { status: "loading", generation: 0 },
        [descendantKey]: { status: "loading", generation: 0 },
        [unrelatedKey]: { status: "loading", generation: 0 },
      },
      pgObjectDescriptionRequestIds: {
        [schemaKey]: 1,
        [descendantKey]: 2,
        [unrelatedKey]: 3,
      },
    });

    useAppStore
      .getState()
      .dropPgObjectDescriptionsForSchema("conn-1", "lifecycle");

    expect(useAppStore.getState().pgObjectDescriptions).toEqual({
      [unrelatedKey]: { status: "loading", generation: 0 },
    });
    expect(useAppStore.getState().pgObjectDescriptionRequestIds).toEqual({
      [schemaKey]: 2,
      [descendantKey]: 3,
      [unrelatedKey]: 3,
    });
  });

  it("keeps the ready description visible while refreshing it", async () => {
    const reference = {
      kind: "sequence",
      schema: "lifecycle",
      name: "order_number_seq",
      identityArgs: null,
    } as const;
    const description: PgObjectDescription = {
      reference,
      owner: "dbunk",
      comment: null,
      definitionSql: "CREATE SEQUENCE lifecycle.order_number_seq;",
      facts: {
        kind: "sequence",
        dataType: "bigint",
        start: "1",
        increment: "1",
        minValue: "1",
        maxValue: "9223372036854775807",
        cycle: false,
        cache: "1",
        lastValue: null,
        ownedBy: null,
      },
    };
    const key = pgObjectDescriptionKey("conn-1", reference);
    useAppStore.setState({
      pgObjectDescriptions: {
        [key]: { status: "ready", generation: 0, description },
      },
    });
    let resolveDescription: ((value: PgObjectDescription) => void) | undefined;
    mockedInvoke.mockImplementationOnce(
      () =>
        new Promise<PgObjectDescription>((resolve) => {
          resolveDescription = resolve;
        }),
    );

    const load = useAppStore
      .getState()
      .loadPgObjectDescription("conn-1", reference, 0);
    expect(useAppStore.getState().pgObjectDescriptions[key]).toEqual({
      status: "loading",
      generation: 0,
      description,
    });

    resolveDescription?.(description);
    await expect(load).resolves.toBe("ready");
  });
});

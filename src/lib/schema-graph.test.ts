import { describe, expect, it } from "vitest";

import {
  ALL_SCHEMAS_SENTINEL,
  buildSchemaGraph,
  filterSchemaRelationshipsForTable,
  type SchemaForeignKey,
  type SchemaTableNode,
  schemaRelationshipsKey,
  tableSchemaMapScope,
} from "@/lib/schema-graph";

const tables: SchemaTableNode[] = [
  {
    schema: "public",
    name: "users",
    columnCount: 4,
    columns: [
      {
        name: "id",
        dataType: "integer",
        nullable: false,
        isPrimaryKey: true,
        ordinalPosition: 1,
      },
      {
        name: "email",
        dataType: "text",
        nullable: false,
        isPrimaryKey: false,
        ordinalPosition: 2,
      },
    ],
  },
  {
    schema: "public",
    name: "orders",
    columnCount: 6,
    columns: [
      {
        name: "id",
        dataType: "integer",
        nullable: false,
        isPrimaryKey: true,
        ordinalPosition: 1,
      },
      {
        name: "user_id",
        dataType: "integer",
        nullable: false,
        isPrimaryKey: false,
        ordinalPosition: 2,
      },
      {
        name: "note",
        dataType: "text",
        nullable: true,
        isPrimaryKey: false,
        ordinalPosition: 3,
      },
    ],
  },
  {
    schema: "public",
    name: "order_items",
    columnCount: 5,
    columns: [
      {
        name: "id",
        dataType: "integer",
        nullable: false,
        isPrimaryKey: true,
        ordinalPosition: 1,
      },
      {
        name: "order_id",
        dataType: "integer",
        nullable: true,
        isPrimaryKey: false,
        ordinalPosition: 2,
      },
      {
        name: "sku",
        dataType: "text",
        nullable: false,
        isPrimaryKey: false,
        ordinalPosition: 3,
      },
    ],
  },
];

const foreignKeys: SchemaForeignKey[] = [
  {
    constraintName: "orders_user_id_fkey",
    fromSchema: "public",
    fromTable: "orders",
    fromColumns: ["user_id"],
    toSchema: "public",
    toTable: "users",
    toColumns: ["id"],
  },
  {
    constraintName: "order_items_order_id_fkey",
    fromSchema: "public",
    fromTable: "order_items",
    fromColumns: ["order_id"],
    toSchema: "public",
    toTable: "orders",
    toColumns: ["id"],
  },
];

describe("schemaRelationshipsKey", () => {
  it("joins connection id and schema with a stable separator", () => {
    expect(schemaRelationshipsKey("conn-1", "public")).toBe("conn-1::public");
  });
});

describe("buildSchemaGraph", () => {
  it("produces one node per table with stable ids", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    expect(graph.nodes).toHaveLength(3);
    const ids = graph.nodes.map((node) => node.id).sort();
    expect(ids).toEqual([
      "public.order_items",
      "public.orders",
      "public.users",
    ]);
  });

  it("labels each node with the table name", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const labels = graph.nodes.map((node) => node.data.label).sort();
    expect(labels).toEqual(["order_items", "orders", "users"]);
  });

  it("carries schema and table metadata on each node so the UI can navigate", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const orders = graph.nodes.find((node) => node.id === "public.orders");
    expect(orders?.data.schema).toBe("public");
    expect(orders?.data.table).toBe("orders");
  });

  it("carries column metadata on table nodes for the ERD renderer", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const users = graph.nodes.find((node) => node.id === "public.users");
    expect(users?.data.columnCount).toBe(4);
    expect(users?.data.columns[0]).toMatchObject({
      name: "id",
      dataType: "integer",
      isPrimaryKey: true,
    });
  });

  it("flags only the active table on its node", () => {
    const graph = buildSchemaGraph(tables, foreignKeys, "orders");
    const flags = Object.fromEntries(
      graph.nodes.map((node) => [node.id, node.data.isActive]),
    );
    expect(flags["public.orders"]).toBe(true);
    expect(flags["public.users"]).toBe(false);
    expect(flags["public.order_items"]).toBe(false);
  });

  it("creates one edge per foreign key with source=from-table, target=to-table", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    expect(graph.edges).toHaveLength(2);
    const edge = graph.edges.find(
      (e) => e.id === "fk:public.orders.orders_user_id_fkey",
    );
    expect(edge?.source).toBe("public.orders");
    expect(edge?.target).toBe("public.users");
  });

  it("collapses multi-column FKs to a single edge", () => {
    const multiColFk: SchemaForeignKey = {
      constraintName: "compound_fk",
      fromSchema: "public",
      fromTable: "order_items",
      fromColumns: ["order_id", "line_no"],
      toSchema: "public",
      toTable: "orders",
      toColumns: ["id", "line_no"],
    };
    const graph = buildSchemaGraph(tables, [multiColFk]);
    expect(graph.edges).toHaveLength(1);
    expect(graph.edges[0].id).toBe("fk:public.order_items.compound_fk");
  });

  it("includes referenced columns on the edge label so users can read the FK", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const edge = graph.edges.find(
      (e) => e.id === "fk:public.orders.orders_user_id_fkey",
    );
    expect(edge?.label).toBe("user_id → id");
    expect(edge?.labelStyle).not.toMatchObject({ display: "none" });
  });

  it("connects FK edges to the expected first-column handles", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const edge = graph.edges.find(
      (e) => e.id === "fk:public.orders.orders_user_id_fkey",
    );
    expect(edge?.sourceHandle).toBe("public.orders.user_id.right");
    expect(edge?.targetHandle).toBe("public.users.id.left");
  });

  it("formats multi-column FK labels with paired column lists", () => {
    const multiColFk: SchemaForeignKey = {
      constraintName: "compound_fk",
      fromSchema: "public",
      fromTable: "order_items",
      fromColumns: ["order_id", "sku"],
      toSchema: "public",
      toTable: "orders",
      toColumns: ["id", "note"],
    };
    const graph = buildSchemaGraph(tables, [multiColFk]);
    expect(graph.edges[0].label).toBe("(order_id, sku) → (id, note)");
  });

  it("carries the backend relationship metadata on each Relationship Edge", () => {
    const fk: SchemaForeignKey = {
      ...foreignKeys[0],
      relationshipType: "foreign key",
      cardinality: "one-to-many",
      cardinalityReason:
        "Referencing columns are not constrained unique on the referencing table",
      onUpdate: "NO ACTION",
      onDelete: "CASCADE",
      fkColumnsNullable: false,
      fkColumnsUnique: false,
      isJunctionParticipant: false,
    };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].data?.foreignKey).toEqual(fk);
  });

  it("marks Junction Table Cards and carries Trigger Indicator metadata", () => {
    const junctionTables: SchemaTableNode[] = [
      {
        ...tables[0],
        isJunctionTable: true,
        triggers: [
          {
            name: "users_audit",
            table: "users",
            columns: ["email"],
            timing: "BEFORE",
            events: ["UPDATE"],
            orientation: "ROW",
            enabled: true,
            functionName: "audit_users",
          },
        ],
      },
    ];
    const graph = buildSchemaGraph(junctionTables, []);
    expect(graph.nodes[0].data.isJunctionTable).toBe(true);
    expect(graph.nodes[0].data.triggers).toHaveLength(1);
    expect(graph.nodes[0].data.triggers[0].name).toBe("users_audit");
  });

  it("defaults junction and trigger metadata when the payload omits them", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const users = graph.nodes.find((node) => node.id === "public.users");
    expect(users?.data.isJunctionTable).toBe(false);
    expect(users?.data.triggers).toEqual([]);
  });

  it("renders crow's-foot at the referencing end for one-to-many cardinality", () => {
    const fk: SchemaForeignKey = {
      ...foreignKeys[0],
      cardinality: "one-to-many",
    };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].markerStart).toBe("url(#crowsfoot-many-start)");
  });

  it("renders a one marker at both ends for one-to-one cardinality with non-null FK columns", () => {
    const fk: SchemaForeignKey = {
      ...foreignKeys[0],
      cardinality: "one-to-one",
      fkColumnsNullable: false,
    };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].markerStart).toBe("url(#crowsfoot-one-start)");
    expect(graph.edges[0].markerEnd).toBe("url(#crowsfoot-one)");
  });

  it("renders zero-or-one at the referenced end when FK columns are nullable", () => {
    const fk: SchemaForeignKey = {
      ...foreignKeys[0],
      cardinality: "one-to-many",
      fkColumnsNullable: true,
    };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].markerEnd).toBe("url(#crowsfoot-zero-or-one)");
  });

  it("renders exactly-one at the referenced end when FK columns are non-null", () => {
    const fk: SchemaForeignKey = {
      ...foreignKeys[0],
      cardinality: "one-to-many",
      fkColumnsNullable: false,
    };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].markerEnd).toBe("url(#crowsfoot-one)");
  });

  it("renders the unknown marker when the backend provides no cardinality", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const edge = graph.edges.find(
      (e) => e.id === "fk:public.orders.orders_user_id_fkey",
    );
    expect(edge?.markerStart).toBe("url(#crowsfoot-unknown-start)");
  });

  it("renders the unknown marker for explicit unknown cardinality", () => {
    const fk: SchemaForeignKey = { ...foreignKeys[0], cardinality: "unknown" };
    const graph = buildSchemaGraph(tables, [fk]);
    expect(graph.edges[0].markerStart).toBe("url(#crowsfoot-unknown-start)");
  });

  it("falls back to column nullability for the referenced-end marker on minimal payloads", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    // orders.user_id is NOT NULL; order_items.order_id is nullable.
    const required = graph.edges.find(
      (e) => e.id === "fk:public.orders.orders_user_id_fkey",
    );
    const optional = graph.edges.find(
      (e) => e.id === "fk:public.order_items.order_items_order_id_fkey",
    );
    expect(required?.markerEnd).toBe("url(#crowsfoot-one)");
    expect(optional?.markerEnd).toBe("url(#crowsfoot-zero-or-one)");
  });

  it("creates a synthetic node for FK targets that are not in the tables list", () => {
    const externalFk: SchemaForeignKey = {
      constraintName: "audit_fk",
      fromSchema: "public",
      fromTable: "orders",
      fromColumns: ["audit_id"],
      toSchema: "audit",
      toTable: "events",
      toColumns: ["id"],
    };
    const graph = buildSchemaGraph(tables, [externalFk]);
    const external = graph.nodes.find((node) => node.id === "audit.events");
    expect(external).toBeDefined();
    expect(external?.data.label).toBe("events");
    expect(external?.data.isExternal).toBe(true);
  });

  it("assigns deterministic positions so layout is stable across renders", () => {
    const a = buildSchemaGraph(tables, foreignKeys);
    const b = buildSchemaGraph(tables, foreignKeys);
    for (const nodeA of a.nodes) {
      const nodeB = b.nodes.find((node) => node.id === nodeA.id);
      expect(nodeB?.position).toEqual(nodeA.position);
    }
  });

  it("honors persisted positions over dagre positions", () => {
    const graph = buildSchemaGraph(tables, foreignKeys, null, {
      positions: { "public.orders": { x: 101, y: 202 } },
    });
    expect(
      graph.nodes.find((node) => node.id === "public.orders")?.position,
    ).toEqual({ x: 101, y: 202 });
  });

  it("filters node columns in keys-only mode", () => {
    const graph = buildSchemaGraph(tables, foreignKeys, null, {
      prefs: { attrMode: "keys-only" },
    });
    const orders = graph.nodes.find((node) => node.id === "public.orders");
    expect(orders?.data.columns.map((column) => column.name)).toEqual([
      "id",
      "user_id",
    ]);
  });

  it("switches routing to step and removes handles in none mode", () => {
    const graph = buildSchemaGraph(tables, foreignKeys, null, {
      prefs: { routing: "step", attrMode: "none" },
    });
    expect(graph.edges[0].type).toBe("step");
    expect(graph.edges[0].sourceHandle).toBeUndefined();
    expect(graph.nodes[0].data.columns).toEqual([]);
  });

  it("returns empty graph when tables and FKs are empty", () => {
    const graph = buildSchemaGraph([], []);
    expect(graph.nodes).toEqual([]);
    expect(graph.edges).toEqual([]);
  });

  it("gives same-named FK constraints on different tables distinct edge ids", () => {
    // PG constraint names are only unique per table.
    const fks: SchemaForeignKey[] = [
      { ...foreignKeys[0], constraintName: "fk_user" },
      {
        ...foreignKeys[1],
        constraintName: "fk_user",
      },
    ];
    const graph = buildSchemaGraph(tables, fks);
    const ids = graph.edges.map((edge) => edge.id);
    expect(new Set(ids).size).toBe(2);
  });

  it("flags only the schema-qualified active table when activeTableId is provided", () => {
    const otherSchemaOrders: SchemaTableNode = {
      schema: "archive",
      name: "orders",
      columnCount: 1,
      columns: [],
    };
    const graph = buildSchemaGraph(
      [...tables, otherSchemaOrders],
      [],
      "orders",
      { activeTableId: "public.orders" },
    );
    const flags = Object.fromEntries(
      graph.nodes.map((node) => [node.id, node.data.isActive]),
    );
    expect(flags["public.orders"]).toBe(true);
    expect(flags["archive.orders"]).toBe(false);
  });
});

describe("tableSchemaMapScope", () => {
  it("cannot collide with real schema scopes or the all-schemas sentinel", () => {
    const scope = tableSchemaMapScope("public", "orders");
    expect(scope).not.toBe("public");
    expect(scope).not.toBe(ALL_SCHEMAS_SENTINEL);
    expect(scope.startsWith("__dbunk:")).toBe(true);
  });

  it("is distinct across tables and schemas", () => {
    expect(tableSchemaMapScope("public", "orders")).not.toBe(
      tableSchemaMapScope("public", "users"),
    );
    expect(tableSchemaMapScope("audit", "orders")).not.toBe(
      tableSchemaMapScope("public", "orders"),
    );
  });
});

describe("filterSchemaRelationshipsForTable", () => {
  const audit: SchemaTableNode = {
    schema: "public",
    name: "audit_events",
    columnCount: 2,
    columns: [],
  };
  const allTables = [...tables, audit];
  const fks: SchemaForeignKey[] = [
    ...foreignKeys,
    {
      constraintName: "users_audit_fkey",
      fromSchema: "public",
      fromTable: "users",
      fromColumns: ["audit_id"],
      toSchema: "public",
      toTable: "audit_events",
      toColumns: ["id"],
    },
  ];

  it("keeps the current Table Card plus directly referencing and referenced Table Cards", () => {
    const scoped = filterSchemaRelationshipsForTable(
      { tables: allTables, foreignKeys: fks },
      "public",
      "orders",
    );
    const names = scoped.tables.map((table) => table.name).sort();
    // order_items references orders (incoming); orders references
    // users (outgoing); both directions are included.
    expect(names).toEqual(["order_items", "orders", "users"]);
  });

  it("excludes second-degree neighbors and their Relationship Edges", () => {
    const scoped = filterSchemaRelationshipsForTable(
      { tables: allTables, foreignKeys: fks },
      "public",
      "orders",
    );
    // audit_events is only reachable through users — second degree.
    expect(scoped.tables.some((table) => table.name === "audit_events")).toBe(
      false,
    );
    expect(
      scoped.foreignKeys.some((fk) => fk.constraintName === "users_audit_fkey"),
    ).toBe(false);
  });

  it("keeps only Relationship Edges incident to the current table", () => {
    const scoped = filterSchemaRelationshipsForTable(
      { tables: allTables, foreignKeys: fks },
      "public",
      "orders",
    );
    expect(scoped.foreignKeys.map((fk) => fk.constraintName).sort()).toEqual([
      "order_items_order_id_fkey",
      "orders_user_id_fkey",
    ]);
  });

  it("keeps self-referencing edges on the current table", () => {
    const selfFk: SchemaForeignKey = {
      constraintName: "orders_parent_fkey",
      fromSchema: "public",
      fromTable: "orders",
      fromColumns: ["parent_id"],
      toSchema: "public",
      toTable: "orders",
      toColumns: ["id"],
    };
    const scoped = filterSchemaRelationshipsForTable(
      { tables: allTables, foreignKeys: [selfFk] },
      "public",
      "orders",
    );
    expect(scoped.foreignKeys).toHaveLength(1);
    expect(scoped.tables.map((table) => table.name)).toEqual(["orders"]);
  });

  it("matches tables schema-qualified so same-named tables in other schemas are excluded", () => {
    const otherSchemaTable: SchemaTableNode = {
      schema: "archive",
      name: "orders",
      columnCount: 1,
      columns: [],
    };
    const scoped = filterSchemaRelationshipsForTable(
      { tables: [...allTables, otherSchemaTable], foreignKeys: fks },
      "public",
      "orders",
    );
    expect(
      scoped.tables.filter((table) => table.name === "orders"),
    ).toHaveLength(1);
    expect(scoped.tables[0]?.schema).toBeDefined();
  });
});

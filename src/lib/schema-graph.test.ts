import { describe, expect, it } from "vitest";

import {
  buildSchemaGraph,
  type SchemaForeignKey,
  type SchemaTableNode,
  schemaRelationshipsKey,
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
    const edge = graph.edges.find((e) => e.id === "fk:orders_user_id_fkey");
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
    expect(graph.edges[0].id).toBe("fk:compound_fk");
  });

  it("includes referenced columns on the edge label so users can read the FK", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const edge = graph.edges.find((e) => e.id === "fk:orders_user_id_fkey");
    expect(edge?.label).toBe("user_id → id");
    expect(edge?.labelStyle).not.toMatchObject({ display: "none" });
  });

  it("connects FK edges to the expected first-column handles", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const edge = graph.edges.find((e) => e.id === "fk:orders_user_id_fkey");
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

  it("assigns crow's foot markers from FK column nullability", () => {
    const graph = buildSchemaGraph(tables, foreignKeys);
    const required = graph.edges.find((e) => e.id === "fk:orders_user_id_fkey");
    const optional = graph.edges.find(
      (e) => e.id === "fk:order_items_order_id_fkey",
    );
    expect(required?.markerStart).toBe("url(#crowsfoot-one)");
    expect(optional?.markerStart).toBe("url(#crowsfoot-zero-or-one)");
    expect(required?.markerEnd).toBe("url(#crowsfoot-many)");
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
});

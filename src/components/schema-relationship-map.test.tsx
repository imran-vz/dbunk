// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { Edge, Node, NodeMouseHandler } from "reactflow";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

// reactflow renders into an SVG/canvas tree that pulls in jsdom unsupported
// APIs. For component-level tests we replace it with a tiny stub that
// renders nodes/edges as DOM nodes we can query and click on.
vi.mock("reactflow", () => {
  type StubNode = Node<{
    label: string;
    columns?: Array<{ name: string }>;
    isActive: boolean;
  }>;
  type StubProps = {
    nodes: StubNode[];
    edges: Edge[];
    onNodeClick?: NodeMouseHandler;
    onNodeDragStop?: (event: React.MouseEvent, node: StubNode) => void;
    nodeTypes?: Record<string, React.ComponentType<{ data: unknown }>>;
    children?: React.ReactNode;
  };
  const Handle = ({
    id,
    type,
    position,
  }: {
    id?: string;
    type: string;
    position: string;
  }) => (
    <span
      data-testid={`schema-flow-handle-${id}`}
      data-position={position}
      data-type={type}
    />
  );
  const ReactFlow = ({
    nodes,
    edges,
    onNodeClick,
    onNodeDragStop,
    nodeTypes,
    children,
  }: StubProps) => (
    <div data-testid="schema-flow">
      {children}
      <div data-testid="schema-flow-nodes">
        {nodes.map((node) => {
          const NodeComponent = nodeTypes?.[node.type ?? ""];
          return (
            <div key={node.id}>
              <button
                type="button"
                data-testid={`schema-flow-node-${node.id}`}
                data-active={node.data?.isActive ? "true" : "false"}
                onClick={(event) =>
                  onNodeClick?.(
                    event as unknown as React.MouseEvent,
                    node as unknown as Node,
                  )
                }
              >
                {NodeComponent ? (
                  <NodeComponent data={node.data} />
                ) : (
                  <>
                    {node.data?.label}
                    {node.data?.columns?.map((column) => (
                      <span key={column.name}>{column.name}</span>
                    ))}
                  </>
                )}
              </button>
              <button
                type="button"
                data-testid={`schema-flow-drag-${node.id}`}
                onClick={(event) =>
                  onNodeDragStop?.(event as unknown as React.MouseEvent, {
                    ...node,
                    position: { x: 77, y: 88 },
                  })
                }
              >
                drag
              </button>
            </div>
          );
        })}
      </div>
      <div data-testid="schema-flow-edges">
        {edges.map((edge) => (
          <div
            key={edge.id}
            data-testid={`schema-flow-edge-${edge.id}`}
            data-source={edge.source}
            data-source-handle={edge.sourceHandle ?? ""}
            data-target={edge.target}
            data-target-handle={edge.targetHandle ?? ""}
            data-marker-start={String(edge.markerStart ?? "")}
            data-marker-end={String(edge.markerEnd ?? "")}
            data-type={edge.type ?? ""}
          >
            {String(edge.label ?? "")}
          </div>
        ))}
      </div>
    </div>
  );
  return {
    __esModule: true,
    default: ReactFlow,
    Background: () => null,
    Controls: () => null,
    Handle,
    MiniMap: () => null,
    Position: { Left: "left", Right: "right" },
    applyNodeChanges: (_changes: unknown, nodes: unknown) => nodes,
  };
});

import {
  SchemaRelationshipMap,
  typeGlyph,
} from "@/components/schema-relationship-map";
import { useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const seedRelationships = () => {
  useAppStore.setState({
    activeConnectionId: "conn-1",
    schemaRelationships: {
      "conn-1::public": {
        tables: [
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
                comment: "Surrogate key",
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
                comment: "Owner account",
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
        ],
        foreignKeys: [
          {
            constraintName: "orders_user_id_fkey",
            fromSchema: "public",
            fromTable: "orders",
            fromColumns: ["user_id"],
            toSchema: "public",
            toTable: "users",
            toColumns: ["id"],
          },
        ],
      },
    },
  });
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("SchemaRelationshipMap", () => {
  it("renders one node per table from the active relationships", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(screen.getByTestId("schema-flow-node-public.users")).toBeTruthy();
    expect(screen.getByTestId("schema-flow-node-public.orders")).toBeTruthy();
  });

  it("passes table columns into schema map nodes", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(
      screen.getByTestId("schema-flow-node-public.orders").textContent,
    ).toContain("user_id");
  });

  it("renders one edge per foreign key", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    const edge = screen.getByTestId("schema-flow-edge-fk:orders_user_id_fkey");
    expect(edge.getAttribute("data-source")).toBe("public.orders");
    expect(edge.getAttribute("data-target")).toBe("public.users");
  });

  it("wires foreign keys to participating column handles", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    const edge = screen.getByTestId("schema-flow-edge-fk:orders_user_id_fkey");
    expect(edge.getAttribute("data-source-handle")).toBe(
      "public.orders.user_id.right",
    );
    expect(edge.getAttribute("data-target-handle")).toBe(
      "public.users.id.left",
    );
    expect(
      screen.getByTestId("schema-flow-handle-public.orders.user_id.right"),
    ).toBeTruthy();
    expect(
      screen.queryByTestId("schema-flow-handle-public.orders.note.right"),
    ).toBeNull();
  });

  it("renders crow's foot markers from child nullability", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    const edge = screen.getByTestId("schema-flow-edge-fk:orders_user_id_fkey");
    expect(edge.getAttribute("data-marker-start")).toBe("url(#crowsfoot-one)");
    expect(edge.getAttribute("data-marker-end")).toBe("url(#crowsfoot-many)");
  });

  it("persists node positions when dragging stops", () => {
    seedRelationships();
    const savePosition = vi.fn(async () => {});
    useAppStore.setState({ saveSchemaMapPosition: savePosition });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(screen.getByTestId("schema-flow-drag-public.orders"));

    expect(savePosition).toHaveBeenCalledWith(
      "conn-1",
      "public",
      "public.orders",
      77,
      88,
    );
  });

  it("applies persisted prefs to routing and node attributes", () => {
    seedRelationships();
    useAppStore.setState({
      schemaMapPrefs: {
        "conn-1": {
          public: {
            routing: "step",
            attrMode: "none",
            showTypes: true,
            showNulls: false,
            showComments: false,
          },
        },
      },
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    const edge = screen.getByTestId("schema-flow-edge-fk:orders_user_id_fkey");
    expect(edge.getAttribute("data-type")).toBe("step");
    expect(edge.getAttribute("data-source-handle")).toBe("");
    expect(
      screen.getByTestId("schema-flow-node-public.orders").textContent,
    ).not.toContain("user_id");
  });

  it("renders comments only when the comments pref is enabled", () => {
    seedRelationships();
    useAppStore.setState({
      schemaMapPrefs: {
        "conn-1": {
          public: {
            routing: "bezier",
            attrMode: "all",
            showTypes: true,
            showNulls: false,
            showComments: true,
          },
        },
      },
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(screen.getByText("Owner account")).toBeTruthy();
  });

  it("flags the active table node with data-active=true", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
      />,
    );

    const active = screen.getByTestId("schema-flow-node-public.orders");
    const inactive = screen.getByTestId("schema-flow-node-public.users");
    expect(active.getAttribute("data-active")).toBe("true");
    expect(inactive.getAttribute("data-active")).toBe("false");
  });

  it("clicking a node opens the matching table tab via focusTableInSchemaMap", () => {
    seedRelationships();
    const focusSpy = vi.fn();
    useAppStore.setState({ focusTableInSchemaMap: focusSpy });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(screen.getByTestId("schema-flow-node-public.orders"));

    expect(focusSpy).toHaveBeenCalledWith("conn-1", "public", "orders");
  });

  it("renders an empty placeholder when no relationships are loaded yet", () => {
    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(screen.getByTestId("schema-flow-empty")).toBeTruthy();
  });

  it("shows an error banner when load failed", () => {
    // Stub the loader so the mount effect does not overwrite our seeded
    // error status with the no-op "idle" path used outside Tauri.
    useAppStore.setState({
      loadSchemaRelationships: vi.fn(async () => {}),
      schemaRelationshipsStatus: {
        "conn-1::public": { state: "error", error: "permission denied" },
      },
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(screen.getByTestId("schema-flow-error").textContent).toContain(
      "permission denied",
    );
  });
});

describe("typeGlyph", () => {
  it("returns the numeric glyph for integer-family types", () => {
    expect(typeGlyph("integer")).toBe("123");
    expect(typeGlyph("bigint")).toBe("123");
    expect(typeGlyph("smallint")).toBe("123");
    expect(typeGlyph("Int64")).toBe("123");
  });

  it("returns the numeric glyph for numeric/decimal/real/double types", () => {
    expect(typeGlyph("numeric")).toBe("123");
    expect(typeGlyph("decimal(18,4)")).toBe("123");
    expect(typeGlyph("real")).toBe("123");
    expect(typeGlyph("double precision")).toBe("123");
  });

  it("returns the boolean glyph for bool-family types", () => {
    expect(typeGlyph("boolean")).toBe("T/F");
    expect(typeGlyph("BOOL")).toBe("T/F");
  });

  it("returns the time glyph for date/time/timestamp types", () => {
    expect(typeGlyph("date")).toBe("time");
    expect(typeGlyph("time")).toBe("time");
    expect(typeGlyph("timestamp")).toBe("time");
    expect(typeGlyph("timestamptz")).toBe("time");
    expect(typeGlyph("DateTime64")).toBe("time");
  });

  it("returns the JSON glyph for json-family types", () => {
    expect(typeGlyph("json")).toBe("{}");
    expect(typeGlyph("jsonb")).toBe("{}");
  });

  it("returns the text glyph for unknown / string types", () => {
    expect(typeGlyph("text")).toBe("A-Z");
    expect(typeGlyph("varchar(64)")).toBe("A-Z");
    expect(typeGlyph("uuid")).toBe("A-Z");
    expect(typeGlyph("")).toBe("A-Z");
  });

  it("is case-insensitive — uppercase input maps the same way as lowercase", () => {
    expect(typeGlyph("INTEGER")).toBe("123");
    expect(typeGlyph("JSONB")).toBe("{}");
    expect(typeGlyph("TIMESTAMP")).toBe("time");
  });
});

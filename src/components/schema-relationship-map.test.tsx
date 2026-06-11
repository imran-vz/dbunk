// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { Edge, Node, NodeMouseHandler } from "@xyflow/react";
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
vi.mock("@xyflow/react", () => {
  type StubNode = Node<{
    label: string;
    columns?: Array<{ name: string }>;
    isActive: boolean;
    isDimmed?: boolean;
  }>;
  type StubEdge = Edge<{ isDimmed?: boolean; isFocused?: boolean }>;
  type StubProps = {
    nodes: StubNode[];
    edges: StubEdge[];
    onNodeClick?: NodeMouseHandler;
    onNodeDoubleClick?: NodeMouseHandler;
    onEdgeClick?: (event: React.MouseEvent, edge: StubEdge) => void;
    onPaneClick?: (event: React.MouseEvent) => void;
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
    onNodeDoubleClick,
    onEdgeClick,
    onPaneClick,
    onNodeDragStop,
    nodeTypes,
    children,
  }: StubProps) => (
    <div data-testid="schema-flow">
      {children}
      <button
        type="button"
        data-testid="schema-flow-pane"
        onClick={(event) => onPaneClick?.(event as unknown as React.MouseEvent)}
      >
        pane
      </button>
      <div data-testid="schema-flow-nodes">
        {nodes.map((node) => {
          const NodeComponent = nodeTypes?.[node.type ?? ""];
          return (
            <div key={node.id}>
              <button
                type="button"
                data-testid={`schema-flow-node-${node.id}`}
                data-active={node.data?.isActive ? "true" : "false"}
                data-dimmed={node.data?.isDimmed ? "true" : "false"}
                onClick={(event) =>
                  onNodeClick?.(
                    event as unknown as React.MouseEvent,
                    node as unknown as Node,
                  )
                }
                onDoubleClick={(event) =>
                  onNodeDoubleClick?.(
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
          <button
            type="button"
            key={edge.id}
            data-testid={`schema-flow-edge-${edge.id}`}
            data-source={edge.source}
            data-source-handle={edge.sourceHandle ?? ""}
            data-target={edge.target}
            data-target-handle={edge.targetHandle ?? ""}
            data-marker-start={String(edge.markerStart ?? "")}
            data-marker-end={String(edge.markerEnd ?? "")}
            data-type={edge.type ?? ""}
            data-dimmed={edge.data?.isDimmed ? "true" : "false"}
            data-focused={edge.data?.isFocused ? "true" : "false"}
            onClick={(event) =>
              onEdgeClick?.(event as unknown as React.MouseEvent, edge)
            }
          >
            {String(edge.label ?? "")}
          </button>
        ))}
      </div>
    </div>
  );
  return {
    __esModule: true,
    default: ReactFlow,
    ReactFlow,
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
import { tableSchemaMapScope } from "@/lib/schema-graph";
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

/**
 * Four tables with the expanded backend metadata:
 *
 *   order_items → orders → users → audit_events
 *
 * `audit_events` is a second-degree neighbor of `orders`. `users`
 * carries Trigger Indicator metadata; `order_items` is a Junction
 * Table Card.
 */
const seedRichRelationships = () => {
  useAppStore.setState({
    activeConnectionId: "conn-1",
    schemaRelationships: {
      "conn-1::public": {
        tables: [
          {
            schema: "public",
            name: "users",
            columnCount: 2,
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
            isJunctionTable: false,
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
          {
            schema: "public",
            name: "orders",
            columnCount: 2,
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
            ],
            isJunctionTable: false,
            triggers: [],
          },
          {
            schema: "public",
            name: "order_items",
            columnCount: 1,
            columns: [
              {
                name: "order_id",
                dataType: "integer",
                nullable: true,
                isPrimaryKey: false,
                ordinalPosition: 1,
              },
            ],
            isJunctionTable: true,
            triggers: [],
          },
          {
            schema: "public",
            name: "audit_events",
            columnCount: 1,
            columns: [
              {
                name: "id",
                dataType: "integer",
                nullable: false,
                isPrimaryKey: true,
                ordinalPosition: 1,
              },
            ],
            isJunctionTable: false,
            triggers: [],
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
            relationshipType: "foreign key",
            cardinality: "one-to-many",
            cardinalityReason:
              "Referencing columns are not constrained unique on the referencing table",
            onUpdate: "NO ACTION",
            onDelete: "CASCADE",
            fkColumnsNullable: false,
            fkColumnsUnique: false,
            isJunctionParticipant: false,
          },
          {
            constraintName: "order_items_order_id_fkey",
            fromSchema: "public",
            fromTable: "order_items",
            fromColumns: ["order_id"],
            toSchema: "public",
            toTable: "orders",
            toColumns: ["id"],
            relationshipType: "foreign key",
            cardinality: "one-to-one",
            cardinalityReason:
              "Referencing columns are constrained unique on the referencing table",
            onUpdate: "NO ACTION",
            onDelete: "NO ACTION",
            fkColumnsNullable: true,
            fkColumnsUnique: true,
            isJunctionParticipant: true,
          },
          {
            constraintName: "users_audit_fkey",
            fromSchema: "public",
            fromTable: "users",
            fromColumns: ["audit_id"],
            toSchema: "public",
            toTable: "audit_events",
            toColumns: ["id"],
            relationshipType: "foreign key",
            cardinality: "one-to-many",
            fkColumnsNullable: true,
            fkColumnsUnique: false,
            isJunctionParticipant: false,
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

    const edge = screen.getByTestId(
      "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
    );
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

    const edge = screen.getByTestId(
      "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
    );
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

  it("renders crow's-foot markers from the backend Relationship Cardinality", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    // one-to-many, non-null FK: crow's foot at the referencing end,
    // exactly-one at the referenced end.
    const oneToMany = screen.getByTestId(
      "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
    );
    expect(oneToMany.getAttribute("data-marker-start")).toBe(
      "url(#crowsfoot-many-start)",
    );
    expect(oneToMany.getAttribute("data-marker-end")).toBe(
      "url(#crowsfoot-one)",
    );

    // one-to-one, nullable FK: one at the referencing end, zero-or-one
    // at the referenced end.
    const oneToOne = screen.getByTestId(
      "schema-flow-edge-fk:public.order_items.order_items_order_id_fkey",
    );
    expect(oneToOne.getAttribute("data-marker-start")).toBe(
      "url(#crowsfoot-one-start)",
    );
    expect(oneToOne.getAttribute("data-marker-end")).toBe(
      "url(#crowsfoot-zero-or-one)",
    );
  });

  it("renders the explicit unknown marker when the payload has no cardinality", () => {
    seedRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    const edge = screen.getByTestId(
      "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
    );
    expect(edge.getAttribute("data-marker-start")).toBe(
      "url(#crowsfoot-unknown-start)",
    );
    expect(edge.getAttribute("data-marker-end")).toBe("url(#crowsfoot-one)");
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

    const edge = screen.getByTestId(
      "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
    );
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

  it("single-clicking a Table Card sets the Focused Table without opening the table", () => {
    seedRichRelationships();
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

    expect(focusSpy).not.toHaveBeenCalled();
    // Focused Table emphasizes itself and direct neighbors; unrelated
    // graph elements dim.
    expect(
      screen
        .getByTestId("schema-flow-node-public.orders")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-node-public.users")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-node-public.order_items")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-node-public.audit_events")
        .getAttribute("data-dimmed"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("schema-flow-edge-fk:public.orders.orders_user_id_fkey")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-edge-fk:public.users.users_audit_fkey")
        .getAttribute("data-dimmed"),
    ).toBe("true");
  });

  it("clicking the empty canvas clears focus and removes dimming", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(screen.getByTestId("schema-flow-node-public.orders"));
    expect(
      screen
        .getByTestId("schema-flow-node-public.audit_events")
        .getAttribute("data-dimmed"),
    ).toBe("true");

    fireEvent.click(screen.getByTestId("schema-flow-pane"));
    expect(
      screen
        .getByTestId("schema-flow-node-public.audit_events")
        .getAttribute("data-dimmed"),
    ).toBe("false");
  });

  it("clears focus and dimming when the map identity changes", () => {
    seedRichRelationships();
    useAppStore.setState((state) => ({
      schemaRelationships: {
        ...state.schemaRelationships,
        "conn-1::audit": {
          tables: [
            { schema: "audit", name: "events", columnCount: 1, columns: [] },
            { schema: "audit", name: "actors", columnCount: 1, columns: [] },
          ],
          foreignKeys: [],
        },
      },
    }));

    const { rerender } = render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );
    fireEvent.click(screen.getByTestId("schema-flow-node-public.orders"));
    expect(
      screen
        .getByTestId("schema-flow-node-public.audit_events")
        .getAttribute("data-dimmed"),
    ).toBe("true");

    rerender(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="audit"
        activeTable={null}
      />,
    );
    expect(
      screen
        .getByTestId("schema-flow-node-audit.events")
        .getAttribute("data-dimmed"),
    ).toBe("false");

    // Returning to the original schema must not resurrect the old
    // Focused Table — the focus was cleared, not merely hidden.
    rerender(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );
    expect(
      screen
        .getByTestId("schema-flow-node-public.audit_events")
        .getAttribute("data-dimmed"),
    ).toBe("false");
  });

  it("double-clicking a Table Card opens the table", () => {
    seedRichRelationships();
    const focusSpy = vi.fn();
    useAppStore.setState({ focusTableInSchemaMap: focusSpy });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.doubleClick(screen.getByTestId("schema-flow-node-public.orders"));

    expect(focusSpy).toHaveBeenCalledWith("conn-1", "public", "orders");
  });

  it("opens the table from the explicit Table Card header action", () => {
    seedRichRelationships();
    const focusSpy = vi.fn();
    useAppStore.setState({ focusTableInSchemaMap: focusSpy });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", { name: "Open table public.orders" }),
    );

    expect(focusSpy).toHaveBeenCalledTimes(1);
    expect(focusSpy).toHaveBeenCalledWith("conn-1", "public", "orders");
  });

  it("dragging a Table Card persists the position without opening the table", () => {
    seedRichRelationships();
    const savePosition = vi.fn(async () => {});
    const focusSpy = vi.fn();
    useAppStore.setState({
      saveSchemaMapPosition: savePosition,
      focusTableInSchemaMap: focusSpy,
    });

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
    expect(focusSpy).not.toHaveBeenCalled();
  });

  it("clicking a Relationship Edge opens the Relationship Detail Popover with backend metadata", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
      ),
    );

    const popover = screen.getByTestId("relationship-detail-popover");
    expect(popover.textContent).toContain("orders_user_id_fkey");
    expect(popover.textContent).toContain("foreign key");
    expect(popover.textContent).toContain("one-to-many");
    expect(popover.textContent).toContain(
      "Referencing columns are not constrained unique on the referencing table",
    );
    expect(popover.textContent).toContain("public.orders (user_id)");
    expect(popover.textContent).toContain("public.users (id)");
    expect(popover.textContent).toContain("CASCADE");
    expect(popover.textContent).toContain("NO ACTION");
  });

  it("shows FK nullability, uniqueness, and junction participation in the popover", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.order_items.order_items_order_id_fkey",
      ),
    );

    const popover = screen.getByTestId("relationship-detail-popover");
    expect(popover.textContent).toContain("one-to-one");
    expect(popover.textContent).toContain("FK columns nullable");
    expect(popover.textContent).toContain("FK columns unique");
    expect(
      screen.getByTestId("relationship-junction-participation"),
    ).toBeTruthy();
  });

  it("keeps trigger metadata out of the Relationship Detail Popover", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    // users carries a trigger; the edge into users must not surface it.
    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
      ),
    );

    const popover = screen.getByTestId("relationship-detail-popover");
    expect(popover.textContent).not.toContain("users_audit");
    expect(popover.textContent).not.toContain("audit_users");
  });

  it("focusing a Relationship Edge emphasizes only its endpoint Table Cards", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
      ),
    );

    expect(
      screen
        .getByTestId("schema-flow-edge-fk:public.orders.orders_user_id_fkey")
        .getAttribute("data-focused"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("schema-flow-node-public.orders")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-node-public.users")
        .getAttribute("data-dimmed"),
    ).toBe("false");
    expect(
      screen
        .getByTestId("schema-flow-node-public.order_items")
        .getAttribute("data-dimmed"),
    ).toBe("true");
    expect(
      screen
        .getByTestId(
          "schema-flow-edge-fk:public.order_items.order_items_order_id_fkey",
        )
        .getAttribute("data-dimmed"),
    ).toBe("true");
  });

  it("closes the Relationship Detail Popover from its close button", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
      ),
    );
    expect(screen.getByTestId("relationship-detail-popover")).toBeTruthy();

    fireEvent.click(
      screen.getByRole("button", { name: "Close relationship details" }),
    );
    expect(screen.queryByTestId("relationship-detail-popover")).toBeNull();
  });

  it("renders Trigger Indicators on the Table Card and its targeted Column Row", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(
      screen.getByTestId("trigger-indicator-table-public.users"),
    ).toBeTruthy();
    expect(
      screen.getByTestId("trigger-indicator-column-public.users.email"),
    ).toBeTruthy();
    expect(
      screen.queryByTestId("trigger-indicator-column-public.users.id"),
    ).toBeNull();
    expect(
      screen.queryByTestId("trigger-indicator-table-public.orders"),
    ).toBeNull();
  });

  it("marks Junction Table Cards with the M:N indicator", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable={null}
      />,
    );

    expect(
      screen.getByTestId("junction-table-indicator-public.order_items"),
    ).toBeTruthy();
    expect(
      screen.queryByTestId("junction-table-indicator-public.users"),
    ).toBeNull();
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

describe("SchemaRelationshipMap in Table-Level Schema Map mode", () => {
  it("shows the current Table Card and direct neighbors, excluding second-degree neighbors", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    // orders plus its incoming (order_items) and outgoing (users)
    // direct neighbors.
    expect(screen.getByTestId("schema-flow-node-public.orders")).toBeTruthy();
    expect(screen.getByTestId("schema-flow-node-public.users")).toBeTruthy();
    expect(
      screen.getByTestId("schema-flow-node-public.order_items"),
    ).toBeTruthy();
    // audit_events hangs off users — second degree, excluded.
    expect(
      screen.queryByTestId("schema-flow-node-public.audit_events"),
    ).toBeNull();
    expect(
      screen.queryByTestId("schema-flow-edge-fk:public.users.users_audit_fkey"),
    ).toBeNull();
  });

  it("persists dragged positions under the dedicated table scope", () => {
    seedRichRelationships();
    const savePosition = vi.fn(async () => {});
    useAppStore.setState({ saveSchemaMapPosition: savePosition });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    fireEvent.click(screen.getByTestId("schema-flow-drag-public.users"));

    expect(savePosition).toHaveBeenCalledWith(
      "conn-1",
      tableSchemaMapScope("public", "orders"),
      "public.users",
      77,
      88,
    );
  });

  it("opens the Relationship Detail Popover from an edge click in table-level mode", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    fireEvent.click(
      screen.getByTestId(
        "schema-flow-edge-fk:public.orders.orders_user_id_fkey",
      ),
    );

    const popover = screen.getByTestId("relationship-detail-popover");
    expect(popover.textContent).toContain("one-to-many");
  });

  it("applies focus and dimming in table-level mode", () => {
    seedRichRelationships();

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    fireEvent.click(screen.getByTestId("schema-flow-node-public.users"));

    // users connects to orders only inside this scope; order_items is
    // unrelated to the Focused Table.
    expect(
      screen
        .getByTestId("schema-flow-node-public.order_items")
        .getAttribute("data-dimmed"),
    ).toBe("true");
    expect(
      screen
        .getByTestId("schema-flow-node-public.orders")
        .getAttribute("data-dimmed"),
    ).toBe("false");
  });

  it("loads positions and prefs under the dedicated table scope", () => {
    seedRichRelationships();
    const loadPositions = vi.fn(async () => {});
    const loadPrefs = vi.fn(async () => {});
    useAppStore.setState({
      loadSchemaMapPositions: loadPositions,
      loadSchemaMapPrefs: loadPrefs,
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    const scope = tableSchemaMapScope("public", "orders");
    expect(loadPositions).toHaveBeenCalledWith("conn-1", scope);
    expect(loadPrefs).toHaveBeenCalledWith("conn-1", scope);
  });

  it("skips refetching schemas that are already loaded or loading", () => {
    seedRichRelationships();
    useAppStore.setState({
      schemaExplorer: {
        "conn-1": [
          { name: "public", tables: [] },
          { name: "billing", tables: [] },
        ],
      },
      schemaRelationshipsStatus: {
        "conn-1::public": { state: "success" },
      },
    });
    const loadRelationships = vi.fn(async () => {});
    useAppStore.setState({ loadSchemaRelationships: loadRelationships });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    // public is cached; only the missing schema is fetched.
    expect(loadRelationships).toHaveBeenCalledTimes(1);
    expect(loadRelationships).toHaveBeenCalledWith("conn-1", "billing");
  });

  it("keeps same-named FK constraints from different tables as distinct Relationship Edges", () => {
    // PG constraint names are only unique per table; a merged map must
    // not let one same-named FK displace another.
    const fkNamed = (fromSchema: string, fromTable: string) => ({
      constraintName: "orders_user_id_fkey",
      fromSchema,
      fromTable,
      fromColumns: ["user_id"],
      toSchema: fromSchema,
      toTable: "users",
      toColumns: ["id"],
    });
    const tenantTables = (schema: string) => [
      { schema, name: "orders", columnCount: 1, columns: [] },
      { schema, name: "users", columnCount: 1, columns: [] },
    ];
    useAppStore.setState({
      activeConnectionId: "conn-1",
      schemaExplorer: {
        "conn-1": [
          { name: "tenant_a", tables: [] },
          { name: "tenant_b", tables: [] },
        ],
      },
      schemaRelationships: {
        "conn-1::tenant_a": {
          tables: tenantTables("tenant_a"),
          foreignKeys: [fkNamed("tenant_a", "orders")],
        },
        "conn-1::tenant_b": {
          tables: tenantTables("tenant_b"),
          foreignKeys: [fkNamed("tenant_b", "orders")],
        },
      },
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="tenant_a"
        activeTable="orders"
        tableScope={{ schema: "tenant_a", table: "orders" }}
      />,
    );

    const edge = screen.getByTestId(
      "schema-flow-edge-fk:tenant_a.orders.orders_user_id_fkey",
    );
    expect(edge.getAttribute("data-source")).toBe("tenant_a.orders");
    expect(edge.getAttribute("data-target")).toBe("tenant_a.users");
  });

  it("renders the partial map with a non-blocking banner when another schema fails", () => {
    seedRichRelationships();
    useAppStore.setState({
      schemaExplorer: {
        "conn-1": [
          { name: "public", tables: [] },
          { name: "billing", tables: [] },
        ],
      },
      schemaRelationshipsStatus: {
        "conn-1::public": { state: "success" },
        "conn-1::billing": { state: "error", error: "permission denied" },
      },
      loadSchemaRelationships: vi.fn(async () => {}),
    });

    render(
      <SchemaRelationshipMap
        connectionId="conn-1"
        schema="public"
        activeTable="orders"
        tableScope={{ schema: "public", table: "orders" }}
      />,
    );

    // The table's own schema rendered; the failure is a banner, not a
    // blank error screen.
    expect(screen.getByTestId("schema-flow-node-public.orders")).toBeTruthy();
    expect(screen.queryByTestId("schema-flow-error")).toBeNull();
    expect(
      screen.getByTestId("schema-flow-partial-error").textContent,
    ).toContain("permission denied");
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

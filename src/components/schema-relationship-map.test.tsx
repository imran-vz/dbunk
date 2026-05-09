// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type { Edge, Node, NodeMouseHandler } from "reactflow";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
}));

// reactflow renders into an SVG/canvas tree that pulls in jsdom unsupported
// APIs. For component-level tests we replace it with a tiny stub that
// renders nodes/edges as DOM nodes we can query and click on.
vi.mock("reactflow", () => {
  type StubNode = Node<{ label: string; isActive: boolean }>;
  type StubProps = {
    nodes: StubNode[];
    edges: Edge[];
    onNodeClick?: NodeMouseHandler;
  };
  const ReactFlow = ({ nodes, edges, onNodeClick }: StubProps) => (
    <div data-testid="schema-flow">
      <div data-testid="schema-flow-nodes">
        {nodes.map((node) => (
          <button
            type="button"
            key={node.id}
            data-testid={`schema-flow-node-${node.id}`}
            data-active={node.data?.isActive ? "true" : "false"}
            onClick={(event) =>
              onNodeClick?.(
                event as unknown as React.MouseEvent,
                node as unknown as Node,
              )
            }
          >
            {node.data?.label}
          </button>
        ))}
      </div>
      <div data-testid="schema-flow-edges">
        {edges.map((edge) => (
          <div
            key={edge.id}
            data-testid={`schema-flow-edge-${edge.id}`}
            data-source={edge.source}
            data-target={edge.target}
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
    MiniMap: () => null,
  };
});

import { SchemaRelationshipMap } from "@/components/schema-relationship-map";
import { useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const seedRelationships = () => {
  useAppStore.setState({
    activeConnectionId: "conn-1",
    schemaRelationships: {
      "conn-1::public": {
        tables: [
          { schema: "public", name: "users", columnCount: 4 },
          { schema: "public", name: "orders", columnCount: 6 },
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

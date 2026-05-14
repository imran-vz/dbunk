// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/components/schema-relationship-map", async () => {
  const React = await import("react");
  return {
    SchemaRelationshipMap: React.forwardRef(
      (
        props: { connectionId: string; schema: string },
        ref: React.ForwardedRef<{ exportImage: () => Promise<void> }>,
      ) => {
        React.useImperativeHandle(ref, () => ({
          exportImage: vi.fn(async () => {}),
        }));
        return (
          <div data-testid="schema-map-mock">
            {props.connectionId}:{props.schema}
          </div>
        );
      },
    ),
  };
});

vi.mock("@/lib/tauri", () => ({
  isTauri: vi.fn(() => false),
  tauriInvoke: vi.fn(),
  errorToMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
}));

import { SchemaMapTab } from "@/components/workspace-overview/schema-map-tab";
import type { Connection } from "@/lib/store";
import { useAppStore } from "@/lib/store";

const initialStoreState = useAppStore.getState();

const connection: Connection = {
  id: "conn-1",
  name: "Primary",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "localhost",
  port: 5432,
  user: "postgres",
  password: "",
  role: "",
  latency: "10 ms",
  ssl: true,
};

beforeEach(() => {
  useAppStore.setState(initialStoreState, true);
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("SchemaMapTab", () => {
  it("defaults to public and persists the selected schema", () => {
    render(
      <SchemaMapTab
        activeConnection={connection}
        schemas={[
          { name: "audit", tables: ["events"] },
          { name: "public", tables: ["users"] },
        ]}
        isClient={false}
      />,
    );

    expect(screen.getByTestId("schema-map-mock").textContent).toBe(
      "conn-1:public",
    );
    expect(useAppStore.getState().connectionSchemaMapSchema["conn-1"]).toBe(
      "public",
    );
  });

  it("resets the selected schema layout from the toolbar", () => {
    const resetSchemaMapPositions = vi.fn(async () => {});
    useAppStore.setState({
      resetSchemaMapPositions,
      connectionSchemaMapSchema: { "conn-1": "audit" },
    });

    render(
      <SchemaMapTab
        activeConnection={connection}
        schemas={[{ name: "audit", tables: ["events"] }]}
        isClient={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /reset/i }));

    expect(resetSchemaMapPositions).toHaveBeenCalledWith("conn-1", "audit");
  });
});

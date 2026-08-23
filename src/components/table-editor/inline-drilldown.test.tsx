/* oxlint-disable anti-slop/no-module-mocking -- The test isolates the shared safety-confirmation boundary. */
// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/invoke-with-safety-confirmation", () => ({
  invokeWithSafetyConfirmation: vi.fn(),
}));

import { InlineDrilldown } from "@/components/table-editor/inline-drilldown";
import { invokeWithSafetyConfirmation } from "@/lib/invoke-with-safety-confirmation";
import { type Connection, useAppStore } from "@/lib/store";

const connection: Connection = {
  id: "conn-1",
  name: "Staging",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "staging.internal",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "10 ms",
  ssl: true,
  environment: "staging",
  safeMode: "protected",
  readOnly: false,
};

const initialStoreState = useAppStore.getState();
const mockedInvoke = vi.mocked(invokeWithSafetyConfirmation);

beforeEach(() => {
  useAppStore.setState(
    { ...initialStoreState, connections: [connection] },
    true,
  );
  mockedInvoke.mockReset();
});

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

describe("InlineDrilldown", () => {
  it("routes FK preview SQL through shared safety confirmation", async () => {
    mockedInvoke.mockResolvedValueOnce({
      columns: ["id", "name"],
      rows: [["7", "Ada"]],
    });

    render(
      <InlineDrilldown
        connectionId={connection.id}
        engine="PostgreSQL"
        target={{ schema: "public", table: "users", column: "id" }}
        value="7"
        onClose={vi.fn()}
      />,
    );

    expect(await screen.findByText("Ada")).toBeTruthy();
    expect(mockedInvoke).toHaveBeenCalledWith({
      command: "run_query",
      connection,
      payload: {
        connectionId: connection.id,
        query: 'SELECT * FROM "public"."users" WHERE "id" = \'7\' LIMIT 5',
      },
    });
  });
});

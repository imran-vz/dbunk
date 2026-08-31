/* oxlint-disable anti-slop/no-module-mocking -- The impact command is mocked so the dialog states are deterministic. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const impactMocks = vi.hoisted(() => ({ load: vi.fn() }));

vi.mock("@/lib/object-ddl", () => ({
  loadPgDropImpact: impactMocks.load,
  formatObjectDdlError: () => "Impact failed.",
}));

import { DropImpactDialog } from "@/components/object-ddl/drop-impact-dialog";
import type { PgObjectRef } from "@/lib/store";

const reference: PgObjectRef = {
  kind: "table",
  schema: "lifecycle",
  name: "orders",
  identityArgs: null,
};

beforeEach(() => impactMocks.load.mockReset());
afterEach(cleanup);

describe("DropImpactDialog", () => {
  it("groups dependents by depth, discloses caps, and requires CASCADE opt-in", async () => {
    impactMocks.load.mockResolvedValue({
      kind: "ok",
      value: {
        dependents: Array.from({ length: 201 }, (_, index) => ({
          objectType: index % 2 === 0 ? "view" : "materialized view",
          identity: `lifecycle.dependent_${index}`,
          depth: index < 100 ? 1 : 2,
        })),
        truncated: true,
      },
    });
    const onReview = vi.fn();

    render(
      <DropImpactDialog
        open
        connectionId="conn-1"
        reference={reference}
        onOpenChange={() => undefined}
        onReview={onReview}
      />,
    );

    expect(await screen.findByText("Depth 1")).toBeTruthy();
    expect(screen.getByText("Depth 2")).toBeTruthy();
    expect(screen.getByText("Showing the first 200 dependents.")).toBeTruthy();
    expect(
      screen.getByText("The server truncated the dependent list."),
    ).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Review drop DDL" }));
    expect(onReview).toHaveBeenLastCalledWith([
      { op: "dropObject", reference, cascade: false },
    ]);

    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: "Review drop DDL" }));
    expect(onReview).toHaveBeenLastCalledWith([
      { op: "dropObject", reference, cascade: true },
    ]);
  });

  it("renders the empty impact state", async () => {
    impactMocks.load.mockResolvedValue({
      kind: "ok",
      value: { dependents: [], truncated: false },
    });

    render(
      <DropImpactDialog
        open
        connectionId="conn-1"
        reference={reference}
        onOpenChange={() => undefined}
        onReview={() => undefined}
      />,
    );

    expect(await screen.findByText("No dependents found.")).toBeTruthy();
    expect(
      screen.getByText("Also drop these 0 dependent objects, CASCADE"),
    ).toBeTruthy();
  });
});

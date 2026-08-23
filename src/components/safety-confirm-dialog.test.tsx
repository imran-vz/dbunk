/* oxlint-disable anti-slop/no-floating-promises -- The unresolved promise is the behavior under test until the dialog confirms it. */
// @vitest-environment jsdom
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SafetyConfirmDialog } from "@/components/safety-confirm-dialog";
import {
  requestSafetyConfirmation,
  resolveSafetyConfirmation,
} from "@/lib/safety-confirmation";
import type { Connection } from "@/lib/store";

const production: Connection = {
  id: "prod",
  name: "Primary",
  database: "app",
  status: "Connected",
  engine: "PostgreSQL",
  host: "prod.internal",
  port: 5432,
  user: "postgres",
  password: "",
  role: "admin",
  latency: "10 ms",
  ssl: true,
  environment: "production",
  safeMode: "inherit",
  readOnly: false,
};

const staging: Connection = {
  ...production,
  id: "staging",
  name: "Staging",
  environment: "staging",
  safeMode: "protected",
};

afterEach(() => {
  resolveSafetyConfirmation(false);
  cleanup();
});

describe("SafetyConfirmDialog", () => {
  it("requires the exact connection name for production-strict operations", async () => {
    render(<SafetyConfirmDialog />);
    let result: Promise<boolean> | undefined;
    act(() => {
      result = requestSafetyConfirmation({
        connection: production,
        subject: {
          kind: "statements",
          statements: [
            {
              index: 0,
              class: "dml",
              unbounded: true,
              destructive: true,
            },
          ],
        },
      });
    });

    const confirm = screen.getByRole("button", { name: "Confirm and run" });
    expect(confirm.hasAttribute("disabled")).toBe(true);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "primary" },
    });
    expect(confirm.hasAttribute("disabled")).toBe(true);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Primary" },
    });
    expect(confirm.hasAttribute("disabled")).toBe(false);
    fireEvent.click(confirm);

    await expect(result).resolves.toBe(true);

    let nextResult: Promise<boolean> | undefined;
    act(() => {
      nextResult = requestSafetyConfirmation({
        connection: staging,
        subject: {
          kind: "command",
          command: "run_pg_restore",
          destructive: true,
        },
      });
    });
    expect(
      screen
        .getByRole("button", { name: "Confirm and run" })
        .hasAttribute("disabled"),
    ).toBe(true);
    expect(screen.getAllByText("Staging").length).toBeGreaterThan(0);
    act(() => resolveSafetyConfirmation(false));
    await expect(nextResult).resolves.toBe(false);
  });
});

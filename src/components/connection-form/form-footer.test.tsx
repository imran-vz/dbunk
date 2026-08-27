/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test harness narrows a bare TanStack form to the field API used by the component. */
// @vitest-environment jsdom
import { useForm } from "@tanstack/react-form";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { FormFooter } from "@/components/connection-form/form-footer";
import { syntheticReachableDiagnosis } from "@/lib/connection-diagnosis";

import {
  buildStoredConnectionFromForm,
  EMPTY_NEW_DEFAULTS,
  type Mode,
} from "./form-utils";
import type { ConnectionFormApi, TestStatus } from "./use-connection-form";

afterEach(cleanup);

function Harness({
  formMode,
  testStatus,
  onTest,
}: {
  formMode: Mode;
  testStatus: TestStatus;
  onTest: () => void;
}) {
  const form = useForm({
    defaultValues: EMPTY_NEW_DEFAULTS,
  }) as unknown as ConnectionFormApi;
  return (
    <FormFooter
      form={form}
      formMode={formMode}
      testStatus={testStatus}
      credentialMode="keychain"
      onTest={onTest}
      handleCancel={() => {}}
    />
  );
}

describe("FormFooter", () => {
  it("offers Test Connection in edit mode too", () => {
    const onTest = vi.fn();
    render(
      <Harness
        formMode="edit"
        testStatus={{ state: "idle" }}
        onTest={onTest}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Test Connection" }));
    expect(onTest).toHaveBeenCalledTimes(1);
  });

  it("renders the outer rejection as a plain error, not a report", () => {
    render(
      <Harness
        formMode="new"
        testStatus={{ state: "error", error: "Connection not found" }}
        onTest={() => {}}
      />,
    );
    expect(screen.getByRole("alert").textContent).toBe("Connection not found");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("renders the diagnosis panel for a finished test", () => {
    const report = syntheticReachableDiagnosis(
      buildStoredConnectionFromForm(EMPTY_NEW_DEFAULTS, "t"),
    );
    render(
      <Harness
        formMode="new"
        testStatus={{ state: "done", report }}
        onTest={() => {}}
      />,
    );
    expect(screen.getByRole("status").textContent).toContain("Reachable");
    expect(
      screen.getByRole("button", { name: "Test Connection" }),
    ).toBeTruthy();
  });
});

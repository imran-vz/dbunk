/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test harness narrows a bare TanStack form to the field API used by the component. */
// @vitest-environment jsdom
import { useForm } from "@tanstack/react-form";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { SafetyFields } from "@/components/connection-form/safety-fields";

import { EMPTY_NEW_DEFAULTS } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

afterEach(cleanup);

function Harness() {
  const form = useForm({
    defaultValues: {
      ...EMPTY_NEW_DEFAULTS,
      environment: "staging" as const,
      safeMode: "strict" as const,
    },
  }) as unknown as ConnectionFormApi;
  return <SafetyFields form={form} />;
}

describe("SafetyFields", () => {
  it("labels Inherit from the environment default, not the selected mode", () => {
    render(<Harness />);

    fireEvent.click(screen.getByRole("combobox", { name: "Safe Mode" }));
    expect(screen.getByText("Inherit (Protected)")).toBeTruthy();
    expect(screen.getByText("Resolves to Strict for Staging.")).toBeTruthy();
  });
});

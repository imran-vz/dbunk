/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test fixtures use controlled mocks and assertions to exercise otherwise inaccessible boundaries. */
// @vitest-environment jsdom
import { useForm } from "@tanstack/react-form";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { DriverOptionsFields } from "@/components/connection-form/driver-options-fields";

import {
  type ConnectionFormData,
  driverOptionsFromForm,
  EMPTY_NEW_DEFAULTS,
} from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

afterEach(() => {
  cleanup();
});

/**
 * Renders the block against a bare form instance and hands the test
 * the instance itself, so assertions read live state rather than a
 * snapshot from the last parent render — only the `form.Field`
 * subscribers re-render on a change, so the parent's copy would go
 * stale. The cast mirrors the shape `useConnectionForm` hands the
 * component; the extra validators there don't change the field API
 * this block touches.
 */
function Harness({
  initial,
  onForm,
}: {
  initial?: Partial<ConnectionFormData>;
  onForm: (form: ConnectionFormApi) => void;
}) {
  const form = useForm({
    defaultValues: { ...EMPTY_NEW_DEFAULTS, ...initial },
  }) as unknown as ConnectionFormApi;
  onForm(form);
  return <DriverOptionsFields form={form} />;
}

function renderFields(initial?: Partial<ConnectionFormData>) {
  let form: ConnectionFormApi | undefined;
  render(
    <Harness
      initial={initial}
      onForm={(next) => {
        form = next;
      }}
    />,
  );
  return () => {
    if (!form) throw new Error("form never mounted");
    return form.state.values;
  };
}

describe("DriverOptionsFields", () => {
  it("renders every knob the backend applies, keepalive included", () => {
    renderFields();
    expect(screen.getByLabelText("Statement timeout (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Idle in transaction (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Connect timeout (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Keepalive idle (seconds)")).toBeTruthy();
    expect(screen.getByLabelText("Search path")).toBeTruthy();
    expect(screen.getByLabelText("Default role")).toBeTruthy();
  });

  it("discloses that keepalive does not reach the pooled driver", () => {
    // ADR-0025: applied on the dedicated driver only. The control must
    // not imply metadata/admin queries are covered.
    renderFields();
    expect(screen.getByText(/pooled driver that cannot set it/i)).toBeTruthy();
  });

  it("round-trips keepalive as a number and clears it to undefined", () => {
    const values = renderFields({ keepaliveSeconds: 30 });
    expect(
      screen.getByLabelText<HTMLInputElement>("Keepalive idle (seconds)").value,
    ).toBe("30");
    fireEvent.change(screen.getByLabelText("Keepalive idle (seconds)"), {
      target: { value: "120" },
    });
    expect(values().keepaliveSeconds).toBe(120);
    fireEvent.change(screen.getByLabelText("Keepalive idle (seconds)"), {
      target: { value: "" },
    });
    expect(values().keepaliveSeconds).toBeUndefined();
  });

  it("writes a typed timeout into form state as a number", () => {
    const values = renderFields();
    fireEvent.change(screen.getByLabelText("Statement timeout (ms)"), {
      target: { value: "30000" },
    });
    expect(values().statementTimeoutMs).toBe(30_000);
  });

  it("clears a timeout back to undefined when the input is emptied", () => {
    // The distinction that matters: "" must mean "use the server
    // default", not 0 — which PG reads as "no limit".
    const values = renderFields({ statementTimeoutMs: 30_000 });
    fireEvent.change(screen.getByLabelText("Statement timeout (ms)"), {
      target: { value: "" },
    });
    expect(values().statementTimeoutMs).toBeUndefined();
    expect(driverOptionsFromForm(values())).toBeUndefined();
  });

  it("hydrates an existing blob into the visible controls", () => {
    renderFields({
      statementTimeoutMs: 30_000,
      defaultSearchPath: "app, public",
      defaultRole: "analyst",
    });
    expect(
      screen.getByLabelText<HTMLInputElement>("Statement timeout (ms)").value,
    ).toBe("30000");
    expect(screen.getByLabelText<HTMLInputElement>("Search path").value).toBe(
      "app, public",
    );
    expect(screen.getByLabelText<HTMLInputElement>("Default role").value).toBe(
      "analyst",
    );
  });
});

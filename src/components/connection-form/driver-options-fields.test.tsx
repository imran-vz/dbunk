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
  it("renders the knobs the backend applies, and no control for keepalive", () => {
    renderFields();
    expect(screen.getByLabelText("Statement timeout (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Idle in transaction (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Connect timeout (ms)")).toBeTruthy();
    expect(screen.getByLabelText("Search path")).toBeTruthy();
    expect(screen.getByLabelText("Default role")).toBeTruthy();
    // sqlx 0.8 has no socket-keepalive setter — the field round-trips
    // through form state but must not present a control that does
    // nothing (ADR-0013 §Decision).
    expect(screen.queryByLabelText(/keepalive/i)).toBeNull();
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

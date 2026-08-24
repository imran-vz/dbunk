/* oxlint-disable anti-slop/require-safety-comment-for-type-assertion, anti-slop/no-known-value-widening, anti-slop/no-unknown-parameters, anti-slop/no-chained-type-assertions -- The fake form records setFieldValue calls; the cast installs it at the component's typed boundary. */
// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { UriImportField } from "@/components/connection-form/uri-import-field";
import type { ConnectionFormApi } from "@/components/connection-form/use-connection-form";

function makeFakeForm() {
  const calls: Array<[string, unknown]> = [];
  const form = {
    setFieldValue: (name: string, value: unknown) => {
      calls.push([name, value]);
    },
  } as unknown as ConnectionFormApi;
  return { form, calls };
}

afterEach(cleanup);

describe("UriImportField (Plan 010, mock A)", () => {
  it("prefills engine-first, then carried fields, from a postgres URI", () => {
    const { form, calls } = makeFakeForm();
    const onEngineChange = vi.fn();
    render(<UriImportField form={form} onEngineChange={onEngineChange} />);

    fireEvent.change(screen.getByLabelText("Import from URI"), {
      target: { value: "postgres://app:pw@10.0.0.1:5433/orders" },
    });

    expect(onEngineChange).toHaveBeenCalledWith("PostgreSQL");
    expect(calls).toEqual([
      ["host", "10.0.0.1"],
      ["port", 5433],
      ["user", "app"],
      ["database", "orders"],
      ["password", "pw"],
    ]);
    expect(screen.getByTestId("uri-import-notice").textContent).toContain(
      "Applied PostgreSQL URI",
    );
  });

  it("maps rediss URIs onto TLS and db-number fields", () => {
    const { form, calls } = makeFakeForm();
    const onEngineChange = vi.fn();
    render(<UriImportField form={form} onEngineChange={onEngineChange} />);

    fireEvent.change(screen.getByLabelText("Import from URI"), {
      target: { value: "rediss://cache.internal/2" },
    });

    expect(onEngineChange).toHaveBeenCalledWith("Redis");
    expect(calls).toContainEqual(["useTls", true]);
    expect(calls).toContainEqual(["dbNumber", 2]);
  });

  it("discloses ignored query parameters", () => {
    const { form } = makeFakeForm();
    render(<UriImportField form={form} onEngineChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText("Import from URI"), {
      target: { value: "postgres://u@h/db?sslmode=require" },
    });

    expect(screen.getByTestId("uri-import-notice").textContent).toContain(
      "sslmode",
    );
  });

  it("shows the parse reason and applies nothing for junk input", () => {
    const { form, calls } = makeFakeForm();
    const onEngineChange = vi.fn();
    render(<UriImportField form={form} onEngineChange={onEngineChange} />);

    fireEvent.change(screen.getByLabelText("Import from URI"), {
      target: { value: "mongodb://nope" },
    });

    expect(onEngineChange).not.toHaveBeenCalled();
    expect(calls).toHaveLength(0);
    expect(screen.getByTestId("uri-import-notice").textContent).toContain(
      "Unsupported scheme",
    );
  });
});

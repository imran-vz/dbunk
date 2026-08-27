/* oxlint-disable anti-slop/no-chained-type-assertions anti-slop/require-safety-comment-for-type-assertion -- Test harness narrows a bare TanStack form to the field API used by the component. */
// @vitest-environment jsdom
import { useForm } from "@tanstack/react-form";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { TlsFields } from "@/components/connection-form/tls-fields";

import { type ConnectionFormData, EMPTY_NEW_DEFAULTS } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

afterEach(cleanup);

function Harness({
  initial,
  onForm,
}: {
  initial?: Partial<ConnectionFormData>;
  onForm?: (form: ConnectionFormApi) => void;
}) {
  const form = useForm({
    defaultValues: { ...EMPTY_NEW_DEFAULTS, ...initial },
  }) as unknown as ConnectionFormApi;
  onForm?.(form);
  return <TlsFields form={form} />;
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
  return {
    values: () => {
      if (!form) throw new Error("form never mounted");
      return form.state.values;
    },
    setMode: (mode: ConnectionFormData["tlsMode"]) => {
      if (!form) throw new Error("form never mounted");
      act(() => {
        form?.setFieldValue("tlsMode", mode);
      });
    },
  };
}

const CA = "CA certificate path";
const CERT = "Client certificate path";
const KEY = "Client key path";
const HOST_NAME = "Certificate host name";

describe("TlsFields", () => {
  it("defaults to prefer and shows only the client-certificate pair", () => {
    renderFields();
    expect(screen.getByRole("combobox", { name: "TLS mode" })).toBeTruthy();
    expect(screen.getByText("Prefer")).toBeTruthy();
    expect(screen.queryByLabelText(CA)).toBeNull();
    expect(screen.getByLabelText(CERT)).toBeTruthy();
    expect(screen.getByLabelText(KEY)).toBeTruthy();
    expect(screen.queryByLabelText(HOST_NAME)).toBeNull();
  });

  it("shows the fields each mode reads and nothing else", () => {
    const { setMode } = renderFields();

    setMode("disable");
    expect(screen.queryByLabelText(CA)).toBeNull();
    expect(screen.queryByLabelText(CERT)).toBeNull();
    expect(screen.queryByLabelText(HOST_NAME)).toBeNull();
    expect(screen.getByText(/never encrypted/)).toBeTruthy();

    setMode("verify-ca");
    expect(screen.getByLabelText(CA)).toBeTruthy();
    expect(screen.getByLabelText(CERT)).toBeTruthy();
    expect(screen.queryByLabelText(HOST_NAME)).toBeNull();

    setMode("verify-full");
    expect(screen.getByLabelText(CA)).toBeTruthy();
    expect(screen.getByLabelText(HOST_NAME)).toBeTruthy();
  });

  it("lists all five libpq modes in the select", () => {
    renderFields();
    fireEvent.click(screen.getByRole("combobox", { name: "TLS mode" }));
    for (const label of [
      "Disable",
      "Prefer (default)",
      "Require",
      "Verify CA",
      "Verify full",
    ]) {
      expect(screen.getByRole("option", { name: label })).toBeTruthy();
    }
  });

  it("writes typed paths into form state", () => {
    const { values } = renderFields({ tlsMode: "verify-full" });
    fireEvent.change(screen.getByLabelText(CA), {
      target: { value: "/etc/ssl/ca.pem" },
    });
    fireEvent.change(screen.getByLabelText(HOST_NAME), {
      target: { value: "db.internal" },
    });
    expect(values()).toMatchObject({
      tlsRootCertPath: "/etc/ssl/ca.pem",
      tlsServerName: "db.internal",
    });
  });

  it("hydrates stored paths into the inputs", () => {
    renderFields({
      tlsMode: "verify-ca",
      tlsRootCertPath: "/ca.pem",
      tlsClientCertPath: "/c.crt",
      tlsClientKeyPath: "/c.key",
    });
    expect(screen.getByLabelText<HTMLInputElement>(CA).value).toBe("/ca.pem");
    expect(screen.getByLabelText<HTMLInputElement>(CERT).value).toBe("/c.crt");
    expect(screen.getByLabelText<HTMLInputElement>(KEY).value).toBe("/c.key");
  });

  it("suggests the typed host as the certificate host name placeholder", () => {
    renderFields({ tlsMode: "verify-full", host: "orders.example.com" });
    expect(screen.getByLabelText<HTMLInputElement>(HOST_NAME).placeholder).toBe(
      "orders.example.com",
    );
  });

  it("advises verification on production connections that do not verify", () => {
    const { setMode } = renderFields({ environment: "production" });
    const advisory = /Production connections should verify/;
    expect(screen.getByText(advisory)).toBeTruthy();

    setMode("require");
    expect(screen.getByText(advisory)).toBeTruthy();

    setMode("verify-ca");
    expect(screen.queryByText(advisory)).toBeNull();

    setMode("verify-full");
    expect(screen.queryByText(advisory)).toBeNull();
  });

  it("does not advise outside production", () => {
    renderFields({ environment: "staging", tlsMode: "prefer" });
    expect(
      screen.queryByText(/Production connections should verify/),
    ).toBeNull();
  });
});

/**
 * PostgreSQL transport-security block (ADR-0025): the libpq TLS mode
 * select and the certificate-path fields each mode reads. Rendered in
 * place of the SSL toggle when the policy's `tlsControls` is
 * `"postgres-modes"`; MySQL keeps `<HostAuthSslField>`.
 *
 * Paths are typed, not picked — no file-dialog plugin is installed and
 * `bastion-form.tsx` set the precedent. The production advisory is copy
 * only; the safety policy (ADR-0021) is the enforcement boundary.
 */

import { IconAlertTriangle } from "@tabler/icons-react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { PG_TLS_MODES, type PgTlsMode } from "@/lib/store/types";

import { FieldError } from "./field-helpers";
import { FIELD_ERROR, tlsModeFields, tlsModeVerifies } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

export const TLS_MODE_META = {
  disable: {
    label: "Disable",
    description: "Plaintext — the connection is never encrypted.",
  },
  prefer: {
    label: "Prefer",
    description: "Encrypt when the server offers it. No certificate check.",
  },
  require: {
    label: "Require",
    description: "Always encrypt. No certificate check.",
  },
  "verify-ca": {
    label: "Verify CA",
    description: "Encrypt and verify the certificate chain.",
  },
  "verify-full": {
    label: "Verify full",
    description: "Verify the chain and that the certificate matches the host.",
  },
} satisfies Record<PgTlsMode, { label: string; description: string }>;

function tlsModeFromValue(value: string | null): PgTlsMode {
  return PG_TLS_MODES.find((mode) => mode === value) ?? "prefer";
}

export function TlsFields({ form }: { form: ConnectionFormApi }) {
  return (
    <div className="grid gap-3 rounded-md border border-border-subtle bg-surface-panel p-3">
      <div className="grid gap-0.5">
        <Label>Transport security</Label>
        <p className="text-2xs text-text-muted">
          How the wire protocol is encrypted and which certificate is trusted.
        </p>
      </div>

      <form.Subscribe
        selector={(state) =>
          [
            state.values.tlsMode ?? "prefer",
            state.values.environment,
            state.values.host,
          ] as const
        }
      >
        {([mode, environment, host]) => {
          const fields = tlsModeFields(mode);
          return (
            <>
              <TlsModeField
                form={form}
                mode={mode}
                advise={environment === "production" && !tlsModeVerifies(mode)}
              />
              {fields.rootCert ? (
                <TlsTextField
                  form={form}
                  name="tlsRootCertPath"
                  id="connection-tls-root-cert"
                  label="CA certificate path"
                  placeholder="/path/to/root-ca.pem"
                  help="Optional — the system trust store is used when empty."
                />
              ) : null}
              {fields.clientCert ? (
                <div className="grid gap-3 sm:grid-cols-2">
                  <TlsTextField
                    form={form}
                    name="tlsClientCertPath"
                    id="connection-tls-client-cert"
                    label="Client certificate path"
                    placeholder="/path/to/client.crt"
                  />
                  <TlsTextField
                    form={form}
                    name="tlsClientKeyPath"
                    id="connection-tls-client-key"
                    label="Client key path"
                    placeholder="/path/to/client.key"
                  />
                  <p className="text-2xs text-text-muted sm:col-span-2">
                    Both or neither. Passphrase-protected keys are not
                    supported.
                  </p>
                </div>
              ) : null}
              {fields.serverName ? (
                <TlsTextField
                  form={form}
                  name="tlsServerName"
                  id="connection-tls-server-name"
                  label="Certificate host name"
                  placeholder={host?.trim() || "db.example.com"}
                  help="Only needed when the host is an IP address or reached through an SSH tunnel whose certificate names a different host."
                />
              ) : null}
            </>
          );
        }}
      </form.Subscribe>
    </div>
  );
}

function TlsModeField({
  form,
  mode,
  advise,
}: {
  form: ConnectionFormApi;
  mode: PgTlsMode;
  advise: boolean;
}) {
  return (
    <form.Field name="tlsMode">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-tls-mode">TLS mode</Label>
          <Select
            value={mode}
            onValueChange={(value) =>
              field.handleChange(tlsModeFromValue(value))
            }
          >
            <SelectTrigger id="connection-tls-mode" className="w-full">
              <SelectValue>{TLS_MODE_META[mode].label}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {PG_TLS_MODES.map((value) => (
                <SelectItem key={value} value={value}>
                  {TLS_MODE_META[value].label}
                  {value === "prefer" ? " (default)" : ""}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p className="m-0 text-2xs text-text-muted">
            {TLS_MODE_META[mode].description}
          </p>
          {advise ? (
            <p
              role="note"
              className="m-0 flex items-start gap-1.5 rounded-md border border-warning/30 bg-warning/10 px-2.5 py-1.5 text-2xs text-warning"
            >
              <IconAlertTriangle className="mt-px size-3 shrink-0" />
              <span>
                Production connections should verify the server certificate.
                Choose Verify CA or Verify full.
              </span>
            </p>
          ) : null}
        </div>
      )}
    </form.Field>
  );
}

type TlsTextFieldName =
  | "tlsRootCertPath"
  | "tlsClientCertPath"
  | "tlsClientKeyPath"
  | "tlsServerName";

function TlsTextField({
  form,
  name,
  id,
  label,
  placeholder,
  help,
}: {
  form: ConnectionFormApi;
  name: TlsTextFieldName;
  id: string;
  label: string;
  placeholder: string;
  help?: string;
}) {
  return (
    <form.Field name={name}>
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor={id}>{label}</Label>
          <Input
            id={id}
            className="font-mono"
            placeholder={placeholder}
            value={field.state.value ?? ""}
            onChange={(event) => field.handleChange(event.target.value)}
            onBlur={field.handleBlur}
          />
          {help ? <p className="text-2xs text-text-muted">{help}</p> : null}
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

/**
 * Redis engine-specific fields. `RedisDbNumberField` renders in place
 * of the shared `database` text input (Redis uses a numbered DB rather
 * than a named one). `RedisAdvancedFields` renders inside the parent's
 * Advanced Options collapsible — TLS toggle + verify-cert toggle.
 */

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { ConnectionFormPolicy } from "@/lib/engine-policy";

import { FieldError, ToggleSwitchRow } from "./field-helpers";
import { FIELD_ERROR } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

interface RedisDbNumberFieldProps {
  form: ConnectionFormApi;
  policy: ConnectionFormPolicy;
}

export function RedisDbNumberField({ form, policy }: RedisDbNumberFieldProps) {
  const maxDbNumber = policy.kind === "redis" ? policy.maxDbNumber : 15;
  const placeholder =
    policy.kind === "redis" ? String(policy.defaultDbNumber) : "0";

  return (
    <form.Field name="dbNumber">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-db-number">DB number</Label>
          <Input
            id="connection-db-number"
            type="number"
            min={0}
            max={maxDbNumber}
            placeholder={placeholder}
            value={field.state.value ?? 0}
            onChange={(event) => field.handleChange(Number(event.target.value))}
            onBlur={field.handleBlur}
          />
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

export function RedisAdvancedFields({ form }: { form: ConnectionFormApi }) {
  return (
    <>
      <form.Field name="useTls">
        {(field) => (
          <ToggleSwitchRow
            id="connection-use-tls"
            title="Use TLS"
            description="Connect over TLS (rediss://)."
            checked={field.state.value ?? false}
            onCheckedChange={(value) => field.handleChange(value)}
          />
        )}
      </form.Field>
      <form.Subscribe selector={(state) => state.values.useTls}>
        {(useTlsValue) =>
          useTlsValue ? (
            <form.Field name="verifyTlsCert">
              {(field) => (
                <ToggleSwitchRow
                  id="connection-verify-tls-cert"
                  title="Verify TLS certificate"
                  description="Disable for self-signed dev servers."
                  checked={field.state.value ?? true}
                  onCheckedChange={(value) => field.handleChange(value)}
                />
              )}
            </form.Field>
          ) : null
        }
      </form.Subscribe>
      <form.Field name="readOnly">
        {(field) => (
          <ToggleSwitchRow
            id="connection-read-only"
            title="Read-only"
            description="Refuse every write on this connection — belt-and-braces safety for production servers (ADR-0009)."
            checked={field.state.value ?? false}
            onCheckedChange={(value) => field.handleChange(value)}
          />
        )}
      </form.Field>
    </>
  );
}

/**
 * Convenience export so callers can import a single `RedisFields`
 * alongside the other engines — though in practice the parent calls
 * `RedisDbNumberField` and `RedisAdvancedFields` separately because
 * they belong to different sections of the form.
 */
export function RedisFields({ form, policy }: RedisDbNumberFieldProps) {
  return (
    <>
      <RedisDbNumberField form={form} policy={policy} />
      <RedisAdvancedFields form={form} />
    </>
  );
}

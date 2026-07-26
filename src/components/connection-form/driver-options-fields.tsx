/**
 * PostgreSQL driver/session knobs — the Advanced-section expander from
 * ADR-0013 Phase 2. The backend has carried `driverOptions` on
 * `PgStoredConnection` since Phase 1 and replays them as `SET`
 * statements on every pooled connect (`postgres/pool.rs`); this is the
 * surface that lets a user set them.
 *
 * Gated by `ConnectionFormPolicy.showDriverOptions` rather than an
 * `engine === "PostgreSQL"` check, per ADR-0012 — MySQL shares the
 * `host-auth` form kind but not the field.
 *
 * `keepaliveSeconds` is intentionally absent: it round-trips through
 * form state so a save can't wipe it, but sqlx 0.8 exposes no socket
 * keepalive setter, so there is nothing for a control to do yet.
 */

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

import { FieldError } from "./field-helpers";
import { FIELD_ERROR } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

export function DriverOptionsFields({ form }: { form: ConnectionFormApi }) {
  return (
    <div className="grid gap-3 rounded-md border border-border-subtle bg-surface-panel p-3">
      <div className="grid gap-0.5">
        <Label>Session defaults</Label>
        <p className="text-[0.6875rem] text-text-muted">
          Applied with SET after every connect. Leave a field blank to use the
          server default.
        </p>
      </div>

      <div className="grid gap-3 sm:grid-cols-2">
        <form.Field name="statementTimeoutMs">
          {(field) => (
            <div className="grid gap-1.5">
              <Label htmlFor="connection-statement-timeout">
                Statement timeout (ms)
              </Label>
              <Input
                id="connection-statement-timeout"
                type="number"
                min={0}
                placeholder="server default"
                value={field.state.value ?? ""}
                onChange={(event) =>
                  field.handleChange(parseOptionalNumber(event.target.value))
                }
                onBlur={field.handleBlur}
              />
              <p className="text-[0.6875rem] text-text-muted">
                Cancels any statement running longer than this. 0 disables the
                limit.
              </p>
              <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
            </div>
          )}
        </form.Field>

        <form.Field name="idleInTransactionTimeoutMs">
          {(field) => (
            <div className="grid gap-1.5">
              <Label htmlFor="connection-idle-in-transaction-timeout">
                Idle in transaction (ms)
              </Label>
              <Input
                id="connection-idle-in-transaction-timeout"
                type="number"
                min={0}
                placeholder="server default"
                value={field.state.value ?? ""}
                onChange={(event) =>
                  field.handleChange(parseOptionalNumber(event.target.value))
                }
                onBlur={field.handleBlur}
              />
              <p className="text-[0.6875rem] text-text-muted">
                Drops sessions holding an open transaction idle this long. 0
                disables the limit.
              </p>
              <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
            </div>
          )}
        </form.Field>
      </div>

      <form.Field name="connectTimeoutMs">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-connect-timeout">
              Connect timeout (ms)
            </Label>
            <Input
              id="connection-connect-timeout"
              type="number"
              min={1}
              placeholder="no limit"
              value={field.state.value ?? ""}
              onChange={(event) =>
                field.handleChange(parseOptionalNumber(event.target.value))
              }
              onBlur={field.handleBlur}
            />
            <p className="text-[0.6875rem] text-text-muted">
              Gives up on an unresponsive server instead of waiting on the OS
              TCP timeout.
            </p>
            <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
          </div>
        )}
      </form.Field>

      <form.Field name="defaultSearchPath">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-default-search-path">Search path</Label>
            <Input
              id="connection-default-search-path"
              className="font-mono"
              placeholder="public"
              value={field.state.value ?? ""}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={field.handleBlur}
            />
            <p className="text-[0.6875rem] text-text-muted">
              Comma-separated schemas, in resolution order. Each entry is quoted
              as written.
            </p>
            <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
          </div>
        )}
      </form.Field>

      <form.Field name="defaultRole">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-default-role">Default role</Label>
            <Input
              id="connection-default-role"
              className="font-mono"
              placeholder="none"
              value={field.state.value ?? ""}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={field.handleBlur}
            />
            <p className="text-[0.6875rem] text-text-muted">
              Postgres role to SET ROLE into after connecting — separate from
              the read/write access level above.
            </p>
            <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
          </div>
        )}
      </form.Field>
    </div>
  );
}

function parseOptionalNumber(value: string): number | undefined {
  if (!value.trim()) {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

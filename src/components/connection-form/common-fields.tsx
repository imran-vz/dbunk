/**
 * Fields shared across multiple engine variants. Splitting them out
 * keeps `<ConnectionForm>` body short and lets the per-engine field
 * components stay focussed on the engine-specific toggles.
 */

import { IconEye, IconEyeOff } from "@tabler/icons-react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type { ConnectionFormPolicy } from "@/lib/engine-policy";
import type { DatabaseEngine } from "@/lib/store";

import { FieldError } from "./field-helpers";
import { FIELD_ERROR, type Mode } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

export function NameField({ form }: { form: ConnectionFormApi }) {
  return (
    <form.Field name="name">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-name">Connection Name</Label>
          <Input
            id="connection-name"
            placeholder="My New Database"
            value={field.state.value}
            onChange={(event) => field.handleChange(event.target.value)}
            onBlur={field.handleBlur}
          />
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

interface EnginePickerFieldProps {
  form: ConnectionFormApi;
  formMode: Mode;
  onEngineChange: (engine: DatabaseEngine) => void;
}

export function EnginePickerField({
  form,
  formMode,
  onEngineChange,
}: EnginePickerFieldProps) {
  return (
    <form.Field name="engine">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-engine">Database Type</Label>
          <Select
            value={field.state.value}
            // SAFETY: The value is constrained by the typed component or library contract at this boundary.
            onValueChange={(value) => onEngineChange(value as DatabaseEngine)}
            disabled={formMode === "edit"}
          >
            <SelectTrigger id="connection-engine" className="w-full">
              <SelectValue placeholder="Select engine" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="PostgreSQL">PostgreSQL</SelectItem>
              <SelectItem value="MySQL">MySQL</SelectItem>
              <SelectItem value="ClickHouse">ClickHouse</SelectItem>
              <SelectItem value="SQLite">SQLite</SelectItem>
              <SelectItem value="Redis">Redis</SelectItem>
            </SelectContent>
          </Select>
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

interface HostPortRowProps {
  form: ConnectionFormApi;
  policy: ConnectionFormPolicy;
}

export function HostPortRow({ form, policy }: HostPortRowProps) {
  const portPlaceholder = computePortPlaceholder(
    policy,
    form.state.values.useHttps ?? false,
  );
  return (
    <div className="grid gap-3 sm:grid-cols-[1fr_7rem]">
      <form.Field name="host">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-host">Host</Label>
            <Input
              id="connection-host"
              placeholder="db.example.com"
              value={field.state.value ?? ""}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={field.handleBlur}
            />
            <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
          </div>
        )}
      </form.Field>
      <form.Field name="port">
        {(field) => (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-port">Port</Label>
            <Input
              id="connection-port"
              type="number"
              placeholder={portPlaceholder}
              value={field.state.value ?? ""}
              onChange={(event) =>
                field.handleChange(Number(event.target.value))
              }
              onBlur={field.handleBlur}
            />
            <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
          </div>
        )}
      </form.Field>
    </div>
  );
}

interface DatabaseFieldProps {
  form: ConnectionFormApi;
  isSqlite: boolean;
}

export function DatabaseField({ form, isSqlite }: DatabaseFieldProps) {
  const label = isSqlite ? "Database file" : "Database";
  const placeholder = isSqlite ? "/path/to/db.sqlite" : "core";
  return (
    <form.Field name="database">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-database">{label}</Label>
          <Input
            id="connection-database"
            placeholder={placeholder}
            value={field.state.value ?? ""}
            onChange={(event) => field.handleChange(event.target.value)}
            onBlur={field.handleBlur}
          />
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

export function UserField({
  form,
  isRedis,
}: {
  form: ConnectionFormApi;
  isRedis: boolean;
}) {
  return (
    <form.Field name="user">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-user">
            {isRedis ? "Username (optional)" : "Username"}
          </Label>
          <Input
            id="connection-user"
            placeholder={isRedis ? "default" : "db_user"}
            value={field.state.value ?? ""}
            onChange={(event) => field.handleChange(event.target.value)}
            onBlur={field.handleBlur}
          />
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

interface PasswordFieldProps {
  form: ConnectionFormApi;
  formMode: Mode;
  isRedis: boolean;
  showPassword: boolean;
  onTogglePassword: () => void;
}

export function PasswordField({
  form,
  formMode,
  isRedis,
  showPassword,
  onTogglePassword,
}: PasswordFieldProps) {
  return (
    <form.Field name="password">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-password">
            {passwordLabel(formMode, isRedis)}
          </Label>
          <div className="relative">
            <Input
              id="connection-password"
              type={showPassword ? "text" : "password"}
              placeholder="••••••••"
              value={field.state.value ?? ""}
              onChange={(event) => field.handleChange(event.target.value)}
              onBlur={field.handleBlur}
              className="pr-9"
            />
            <button
              type="button"
              aria-label={showPassword ? "Hide password" : "Show password"}
              onClick={onTogglePassword}
              className="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1 text-text-muted hover:text-foreground"
            >
              {showPassword ? (
                <IconEyeOff className="size-3.5" />
              ) : (
                <IconEye className="size-3.5" />
              )}
            </button>
          </div>
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

export function RoleField({ form }: { form: ConnectionFormApi }) {
  return (
    <form.Field name="role">
      {(field) => (
        <div className="grid gap-1.5">
          <Label htmlFor="connection-role">Role</Label>
          <Input
            id="connection-role"
            placeholder="read/write"
            value={field.state.value ?? ""}
            onChange={(event) => field.handleChange(event.target.value)}
            onBlur={field.handleBlur}
          />
          <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
        </div>
      )}
    </form.Field>
  );
}

function passwordLabel(formMode: Mode, isRedis: boolean): string {
  if (isRedis) return "Password (optional)";
  if (formMode === "edit") return "Password (leave blank to keep existing)";
  return "Password";
}

function computePortPlaceholder(
  policy: ConnectionFormPolicy,
  useHttps: boolean,
): string {
  switch (policy.kind) {
    case "host-auth":
      return String(policy.defaultPort);
    case "redis":
      return String(policy.defaultPort);
    case "clickhouse-http":
      // Switches based on the form's live `useHttps`; the advanced
      // section toggles re-render the field with the new placeholder.
      return useHttps
        ? String(policy.defaultPortHttps)
        : String(policy.defaultPortHttp);
    case "file":
      return "";
  }
}

import {
  IconChevronDown,
  IconChevronRight,
  IconEye,
  IconEyeOff,
  IconShieldLock,
} from "@tabler/icons-react";
import { useForm } from "@tanstack/react-form";
import { useState } from "react";
import * as z from "zod";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { type DatabaseEngine, useAppStore } from "@/lib/store";

const connectionSchema = z
  .object({
    name: z.string().min(1, "Connection name is required"),
    engine: z.enum(["PostgreSQL", "MySQL", "ClickHouse", "SQLite"]),
    host: z.string().optional(),
    database: z.string().min(1, "Database is required"),
    port: z.number().int().optional(),
    user: z.string().optional(),
    password: z.string().optional(),
    role: z.string().optional(),
    ssl: z.boolean().optional(),
    useHttps: z.boolean().optional(),
    urlPath: z.string().optional(),
  })
  .superRefine((value, ctx) => {
    if (value.engine === "SQLite") {
      return;
    }
    if (!value.host?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Host is required",
        path: ["host"],
      });
    }
    if (!value.port || value.port < 1 || value.port > 65535) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Port must be between 1 and 65535",
        path: ["port"],
      });
    }
    if (!value.user?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "User is required",
        path: ["user"],
      });
    }
    if (!value.password?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Password is required",
        path: ["password"],
      });
    }
  });

type ConnectionFormData = z.infer<typeof connectionSchema>;

export interface NewConnectionFormProps {
  /**
   * Called after a successful save. The form clears itself and resets engine
   * to PostgreSQL — the parent controls dialog/sheet visibility.
   */
  onSaved?: () => void;
  onCancel?: () => void;
}

const FIELD_ERROR = (
  errors: Array<{ message?: string } | string | undefined> | undefined,
): string | null => {
  const first = errors?.[0];
  if (!first) return null;
  if (typeof first === "string") return first;
  return first.message ?? null;
};

export function NewConnectionForm({
  onSaved,
  onCancel,
}: NewConnectionFormProps) {
  const [selectedEngine, setSelectedEngine] =
    useState<DatabaseEngine>("PostgreSQL");
  const [useHttps, setUseHttps] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [testStatus, setTestStatus] = useState<
    | { state: "idle" }
    | { state: "running" }
    | { state: "success"; latencyMs: number }
    | { state: "error"; error: string }
  >({ state: "idle" });
  const addConnection = useAppStore((state) => state.addConnection);
  const testConnection = useAppStore((state) => state.testConnection);
  const credentialMode = useAppStore(
    (state) => state.appSettings?.credentialStorageMode,
  );

  const isSQLite = selectedEngine === "SQLite";
  const isClickHouse = selectedEngine === "ClickHouse";
  const portPlaceholder =
    selectedEngine === "MySQL"
      ? "3306"
      : selectedEngine === "ClickHouse"
        ? useHttps
          ? "8443"
          : "8123"
        : "5432";
  const databasePlaceholder =
    selectedEngine === "SQLite" ? "/path/to/db.sqlite" : "core";
  const databaseLabel =
    selectedEngine === "SQLite" ? "Database file" : "Database";

  const form = useForm({
    defaultValues: {
      name: "",
      engine: "PostgreSQL",
      host: "",
      database: "",
      port: 5432,
      user: "",
      password: "",
      role: "read/write",
      ssl: true,
      useHttps: false,
      urlPath: "",
    } as ConnectionFormData,
    onSubmit: async ({ value }) => {
      await addConnection({
        id: crypto.randomUUID(),
        name: value.name,
        database: value.database,
        status: "Disconnected",
        engine: value.engine,
        host: value.host ?? "",
        port: value.port ?? 0,
        user: value.user ?? "",
        password: value.password ?? "",
        role: value.role || "read/write",
        latency: "--",
        lastSync: "Never",
        useHttps:
          value.engine === "ClickHouse" ? (value.useHttps ?? false) : false,
        urlPath: value.engine === "ClickHouse" ? (value.urlPath ?? "") : "",
      });
      form.reset();
      setSelectedEngine("PostgreSQL");
      setUseHttps(false);
      setShowPassword(false);
      onSaved?.();
    },
    validators: {
      onChange: connectionSchema,
    },
  });

  const handleCancel = () => {
    form.reset();
    setSelectedEngine("PostgreSQL");
    setUseHttps(false);
    setShowPassword(false);
    setTestStatus({ state: "idle" });
    onCancel?.();
  };

  return (
    <form
      className="flex h-full min-h-0 flex-col"
      onSubmit={(event) => {
        event.preventDefault();
        event.stopPropagation();
        form.handleSubmit();
      }}
    >
      <div className="flex flex-1 min-h-0 flex-col gap-4 overflow-auto p-4">
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

        <form.Field name="engine">
          {(field) => (
            <div className="grid gap-1.5">
              <Label htmlFor="connection-engine">Database Type</Label>
              <Select
                value={field.state.value}
                onValueChange={(value) => {
                  const engine = value as DatabaseEngine;
                  field.handleChange(engine);
                  setSelectedEngine(engine);
                }}
              >
                <SelectTrigger id="connection-engine" className="w-full">
                  <SelectValue placeholder="Select engine" />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="PostgreSQL">PostgreSQL</SelectItem>
                  <SelectItem value="MySQL">MySQL</SelectItem>
                  <SelectItem value="ClickHouse">ClickHouse</SelectItem>
                  <SelectItem value="SQLite">SQLite</SelectItem>
                </SelectContent>
              </Select>
              <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
            </div>
          )}
        </form.Field>

        {!isSQLite ? (
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
        ) : null}

        <form.Field name="database">
          {(field) => (
            <div className="grid gap-1.5">
              <Label htmlFor="connection-database">{databaseLabel}</Label>
              <Input
                id="connection-database"
                placeholder={databasePlaceholder}
                value={field.state.value}
                onChange={(event) => field.handleChange(event.target.value)}
                onBlur={field.handleBlur}
              />
              <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
            </div>
          )}
        </form.Field>

        {!isSQLite ? (
          <>
            <form.Field name="user">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-user">Username</Label>
                  <Input
                    id="connection-user"
                    placeholder="db_user"
                    value={field.state.value ?? ""}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                  <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
                </div>
              )}
            </form.Field>

            <form.Field name="password">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-password">Password</Label>
                  <div className="relative">
                    <Input
                      id="connection-password"
                      type={showPassword ? "text" : "password"}
                      placeholder="••••••••"
                      value={field.state.value ?? ""}
                      onChange={(event) =>
                        field.handleChange(event.target.value)
                      }
                      onBlur={field.handleBlur}
                      className="pr-9"
                    />
                    <button
                      type="button"
                      aria-label={
                        showPassword ? "Hide password" : "Show password"
                      }
                      onClick={() => setShowPassword((prev) => !prev)}
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

            <form.Field name="ssl">
              {(field) => (
                <label
                  htmlFor="connection-ssl"
                  className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
                >
                  <span className="flex flex-col gap-0.5">
                    <span className="text-xs font-medium">SSL</span>
                    <span className="text-[0.6875rem] text-text-muted">
                      Enable SSL/TLS
                    </span>
                  </span>
                  <Switch
                    id="connection-ssl"
                    checked={field.state.value ?? false}
                    onCheckedChange={(value) => field.handleChange(value)}
                  />
                </label>
              )}
            </form.Field>

            <button
              type="button"
              onClick={() => setAdvancedOpen((prev) => !prev)}
              className="flex items-center gap-1 self-start text-xs font-medium text-text-secondary hover:text-foreground"
            >
              {advancedOpen ? (
                <IconChevronDown className="size-3.5" />
              ) : (
                <IconChevronRight className="size-3.5" />
              )}
              Advanced Options
            </button>
            {advancedOpen ? (
              <>
                <form.Field name="role">
                  {(field) => (
                    <div className="grid gap-1.5">
                      <Label htmlFor="connection-role">Role</Label>
                      <Input
                        id="connection-role"
                        placeholder="read/write"
                        value={field.state.value ?? ""}
                        onChange={(event) =>
                          field.handleChange(event.target.value)
                        }
                        onBlur={field.handleBlur}
                      />
                      <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
                    </div>
                  )}
                </form.Field>
                {isClickHouse ? (
                  <>
                    <form.Field name="useHttps">
                      {(field) => (
                        <label
                          htmlFor="connection-use-https"
                          className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
                        >
                          <span className="flex flex-col gap-0.5">
                            <span className="text-xs font-medium">
                              Use HTTPS
                            </span>
                            <span className="text-[0.6875rem] text-text-muted">
                              Connect to ClickHouse over TLS (port 8443).
                            </span>
                          </span>
                          <Switch
                            id="connection-use-https"
                            checked={field.state.value ?? false}
                            onCheckedChange={(value) => {
                              field.handleChange(value);
                              setUseHttps(value);
                            }}
                          />
                        </label>
                      )}
                    </form.Field>
                    <form.Field name="urlPath">
                      {(field) => (
                        <div className="grid gap-1.5">
                          <Label htmlFor="connection-url-path">URL path</Label>
                          <Input
                            id="connection-url-path"
                            placeholder="/clickhouse (optional)"
                            value={field.state.value ?? ""}
                            onChange={(event) =>
                              field.handleChange(event.target.value)
                            }
                            onBlur={field.handleBlur}
                          />
                        </div>
                      )}
                    </form.Field>
                  </>
                ) : null}
              </>
            ) : null}
          </>
        ) : null}
      </div>

      <div className="flex flex-col gap-2 border-t border-border-subtle bg-surface-window p-4">
        {testStatus.state === "success" ? (
          <div className="rounded-md border border-accent-green/30 bg-accent-green/10 px-3 py-2 text-[0.6875rem] text-accent-green-hover">
            Connected in {testStatus.latencyMs} ms.
          </div>
        ) : null}
        {testStatus.state === "error" ? (
          <div
            role="alert"
            className="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 font-mono text-[0.6875rem] text-danger"
          >
            {testStatus.error}
          </div>
        ) : null}
        <div className="flex gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="flex-1"
            disabled={testStatus.state === "running"}
            onClick={async () => {
              const value = form.state.values;
              setTestStatus({ state: "running" });
              const result = await testConnection({
                id: "test-connection",
                name: value.name || "Untitled",
                database: value.database,
                engine: value.engine,
                host: value.host ?? "",
                port: value.port ?? 0,
                user: value.user ?? "",
                password: value.password ?? "",
                role: value.role || "read/write",
                useHttps:
                  value.engine === "ClickHouse"
                    ? (value.useHttps ?? false)
                    : false,
                urlPath:
                  value.engine === "ClickHouse" ? (value.urlPath ?? "") : "",
              });
              if (result.ok) {
                setTestStatus({
                  state: "success",
                  latencyMs: result.latencyMs,
                });
              } else {
                setTestStatus({ state: "error", error: result.error });
              }
            }}
          >
            {testStatus.state === "running" ? "Testing…" : "Test Connection"}
          </Button>
          <Button
            type="submit"
            size="sm"
            className="flex-1"
            disabled={form.state.isSubmitting || !form.state.isValid}
          >
            {form.state.isSubmitting ? "Saving…" : "Create Connection"}
          </Button>
        </div>
        <div className="flex items-center gap-1.5 text-[0.6875rem] text-text-muted">
          <IconShieldLock className="size-3 text-accent-green" />
          {credentialMode === "plain-sqlite"
            ? "Credentials are stored in the local SQLite database without encryption."
            : credentialMode === "keychain"
              ? "Credentials are stored with the OS keychain."
              : "Credentials are encrypted in the local SQLite database."}
        </div>
        {onCancel ? (
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="self-start text-text-muted"
            onClick={handleCancel}
          >
            Cancel
          </Button>
        ) : null}
      </div>
    </form>
  );
}

function FieldError({ text }: { text: string | null }) {
  if (!text) return null;
  return <p className="text-[0.6875rem] text-danger">{text}</p>;
}

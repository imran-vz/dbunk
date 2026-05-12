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
import { connectionFormPolicy, validateConnection } from "@/lib/engine-policy";
import {
  type Connection,
  type DatabaseEngine,
  type StoredConnection,
  useAppStore,
} from "@/lib/store";

const connectionSchema = z
  .object({
    name: z.string().min(1, "Connection name is required"),
    engine: z.enum(["PostgreSQL", "MySQL", "ClickHouse", "SQLite", "Redis"]),
    host: z.string().optional(),
    database: z.string().optional(),
    port: z.number().int().optional(),
    user: z.string().optional(),
    password: z.string().optional(),
    role: z.string().optional(),
    ssl: z.boolean().optional(),
    useHttps: z.boolean().optional(),
    urlPath: z.string().optional(),
    dbNumber: z.number().int().min(0).max(15).optional(),
    useTls: z.boolean().optional(),
    verifyTlsCert: z.boolean().optional(),
  })
  .superRefine((value, ctx) => {
    // Per-kind validation lives next to the engine policy
    // (`engine-policy.ts`). The schema only owns shape; semantic rules
    // are pulled from the shared validator so the new and edit forms
    // can't drift apart again. See ADR-0012.
    const issues = validateConnection(
      connectionFormPolicy(value.engine),
      value,
      "new",
    );
    for (const issue of issues) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: issue.message,
        path: [issue.path],
      });
    }
  });

type ConnectionFormData = z.infer<typeof connectionSchema>;

/**
 * Project the form values into the right `StoredConnection` variant.
 * Slice 4 (#16) lifts this projection into the unified `ConnectionForm`
 * component alongside the per-kind validator.
 */
function buildStoredConnectionFromForm(
  value: ConnectionFormData,
  id: string,
): StoredConnection {
  const common = {
    id,
    name: value.name,
    database: value.database ?? "",
    host: value.host ?? "",
    port: value.port ?? 0,
    user: value.user ?? "",
    password: value.password ?? "",
    role: value.role || "read/write",
  };
  switch (value.engine) {
    case "PostgreSQL":
    case "MySQL":
      return { ...common, engine: value.engine, ssl: value.ssl ?? true };
    case "SQLite":
      return { ...common, engine: "SQLite" };
    case "ClickHouse":
      return {
        ...common,
        engine: "ClickHouse",
        useHttps: value.useHttps ?? false,
        urlPath: value.urlPath ?? "",
      };
    case "Redis":
      return {
        ...common,
        engine: "Redis",
        dbNumber: value.dbNumber ?? 0,
        useTls: value.useTls ?? false,
        verifyTlsCert: value.verifyTlsCert ?? true,
      };
  }
}

function buildConnectionFromForm(value: ConnectionFormData): Connection {
  return {
    ...buildStoredConnectionFromForm(value, crypto.randomUUID()),
    status: "Disconnected",
    latency: "--",
    lastSync: "Never",
  };
}

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
  const isRedis = selectedEngine === "Redis";
  const portPlaceholder =
    selectedEngine === "MySQL"
      ? "3306"
      : selectedEngine === "ClickHouse"
        ? useHttps
          ? "8443"
          : "8123"
        : selectedEngine === "Redis"
          ? "6379"
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
      dbNumber: 0,
      useTls: false,
      verifyTlsCert: true,
    } as ConnectionFormData,
    onSubmit: async ({ value }) => {
      await addConnection(buildConnectionFromForm(value));
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
                  <SelectItem value="Redis">Redis</SelectItem>
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

        {!isRedis ? (
          <form.Field name="database">
            {(field) => (
              <div className="grid gap-1.5">
                <Label htmlFor="connection-database">{databaseLabel}</Label>
                <Input
                  id="connection-database"
                  placeholder={databasePlaceholder}
                  value={field.state.value ?? ""}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
                <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
              </div>
            )}
          </form.Field>
        ) : (
          <form.Field name="dbNumber">
            {(field) => (
              <div className="grid gap-1.5">
                <Label htmlFor="connection-db-number">DB number</Label>
                <Input
                  id="connection-db-number"
                  type="number"
                  min={0}
                  max={15}
                  placeholder="0"
                  value={field.state.value ?? 0}
                  onChange={(event) =>
                    field.handleChange(Number(event.target.value))
                  }
                  onBlur={field.handleBlur}
                />
                <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
              </div>
            )}
          </form.Field>
        )}

        {!isSQLite ? (
          <>
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

            <form.Field name="password">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-password">
                    {isRedis ? "Password (optional)" : "Password"}
                  </Label>
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

            {/* PG/MySQL SSL toggle. Redis has its own TLS toggle under
              Advanced Options below; ClickHouse uses `Use HTTPS`. */}
            {!isRedis && !isClickHouse ? (
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
            ) : null}

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
                {isRedis ? (
                  <>
                    <form.Field name="useTls">
                      {(field) => (
                        <label
                          htmlFor="connection-use-tls"
                          className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
                        >
                          <span className="flex flex-col gap-0.5">
                            <span className="text-xs font-medium">Use TLS</span>
                            <span className="text-[0.6875rem] text-text-muted">
                              Connect over TLS (rediss://).
                            </span>
                          </span>
                          <Switch
                            id="connection-use-tls"
                            checked={field.state.value ?? false}
                            onCheckedChange={(value) =>
                              field.handleChange(value)
                            }
                          />
                        </label>
                      )}
                    </form.Field>
                    <form.Subscribe selector={(state) => state.values.useTls}>
                      {(useTlsValue) =>
                        useTlsValue ? (
                          <form.Field name="verifyTlsCert">
                            {(field) => (
                              <label
                                htmlFor="connection-verify-tls-cert"
                                className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
                              >
                                <span className="flex flex-col gap-0.5">
                                  <span className="text-xs font-medium">
                                    Verify TLS certificate
                                  </span>
                                  <span className="text-[0.6875rem] text-text-muted">
                                    Disable for self-signed dev servers.
                                  </span>
                                </span>
                                <Switch
                                  id="connection-verify-tls-cert"
                                  checked={field.state.value ?? true}
                                  onCheckedChange={(value) =>
                                    field.handleChange(value)
                                  }
                                />
                              </label>
                            )}
                          </form.Field>
                        ) : null
                      }
                    </form.Subscribe>
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
              const result = await testConnection(
                buildStoredConnectionFromForm(value, "test-connection"),
              );
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

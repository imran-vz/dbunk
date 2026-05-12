/**
 * Unified ConnectionForm — the single form component for both creating
 * a new Connection and editing an existing one. The `mode` prop forks
 * the user-visible behavior:
 *
 * - `mode: "new"` — fresh connection. Engine picker is editable;
 *   common fields (name/host/port/user/password) carry across
 *   engine-switches; the Test Connection button + credential-storage
 *   hint render in the footer.
 * - `mode: "edit"` — existing connection. Engine picker is disabled
 *   (changing engine on a tagged-union record is a delete-and-recreate,
 *   not an edit). Footer is just Cancel + Save changes.
 *
 * Field rendering is policy-driven (`connectionFormPolicy(engine)`):
 * the form switches on `policy.kind` to decide which engine-specific
 * fields appear (`ssl` for host-auth, `useHttps`/`urlPath` for
 * clickhouse-http, `dbNumber`/`useTls`/`verifyTlsCert` for redis,
 * nothing for file). Validation is delegated to `validateConnection`
 * — one shared validator, mode-aware password rule.
 *
 * See ADR-0012 for the unified-form decision and ADR-0010 for the
 * `ssl` wiring this form makes user-editable.
 */

import {
  IconChevronDown,
  IconChevronRight,
  IconEye,
  IconEyeOff,
  IconShieldLock,
} from "@tabler/icons-react";
import { useForm } from "@tanstack/react-form";
import { useEffect, useState } from "react";
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

const connectionSchema = z.object({
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
});

type ConnectionFormData = z.infer<typeof connectionSchema>;

/**
 * Project form values into the right `StoredConnection` variant. The
 * switch on `engine` is the single construction site for the wire
 * shape — Slice 4 collapses the previously-duplicated builders in
 * new-connection-form + edit-connection-dialog into this one place.
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

function buildConnectionFromForm(
  value: ConnectionFormData,
  id: string,
  runtime: {
    status: Connection["status"];
    latency: string;
    lastSync: string;
    errorMessage?: string;
    lastActivityAt?: string;
  },
): Connection {
  return { ...buildStoredConnectionFromForm(value, id), ...runtime };
}

function defaultValuesFromConnection(
  connection: Connection,
): ConnectionFormData {
  const common = {
    name: connection.name,
    engine: connection.engine,
    host: connection.host,
    database: connection.database,
    port: connection.port || 5432,
    user: connection.user,
    password: "",
    role: connection.role,
  };
  switch (connection.engine) {
    case "PostgreSQL":
    case "MySQL":
      return {
        ...common,
        ssl: connection.ssl,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
      };
    case "SQLite":
      return {
        ...common,
        ssl: true,
        useHttps: false,
        urlPath: "",
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
      };
    case "ClickHouse":
      return {
        ...common,
        ssl: true,
        useHttps: connection.useHttps,
        urlPath: connection.urlPath,
        dbNumber: 0,
        useTls: false,
        verifyTlsCert: true,
      };
    case "Redis":
      return {
        ...common,
        ssl: true,
        useHttps: false,
        urlPath: "",
        dbNumber: connection.dbNumber,
        useTls: connection.useTls,
        verifyTlsCert: connection.verifyTlsCert,
      };
  }
}

const EMPTY_NEW_DEFAULTS: ConnectionFormData = {
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
};

type Mode = "new" | "edit";

export interface ConnectionFormProps {
  mode: Mode;
  /** Required when `mode === "edit"`; ignored otherwise. */
  connection?: Connection;
  /** Called after a successful save. Caller controls dialog dismiss. */
  onSaved?: () => void;
  /** Called when the user clicks Cancel. Caller controls dialog dismiss. */
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

export function ConnectionForm({
  mode,
  connection,
  onSaved,
  onCancel,
}: ConnectionFormProps) {
  const initial =
    mode === "edit" && connection
      ? defaultValuesFromConnection(connection)
      : EMPTY_NEW_DEFAULTS;

  const [selectedEngine, setSelectedEngine] = useState<DatabaseEngine>(
    initial.engine,
  );
  const [showPassword, setShowPassword] = useState(false);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [testStatus, setTestStatus] = useState<
    | { state: "idle" }
    | { state: "running" }
    | { state: "success"; latencyMs: number }
    | { state: "error"; error: string }
  >({ state: "idle" });

  const addConnection = useAppStore((state) => state.addConnection);
  const updateConnection = useAppStore((state) => state.updateConnection);
  const testConnection = useAppStore((state) => state.testConnection);
  const credentialMode = useAppStore(
    (state) => state.appSettings?.credentialStorageMode,
  );

  const policy = connectionFormPolicy(selectedEngine);
  const formMode = mode;

  const form = useForm({
    defaultValues: initial,
    onSubmit: async ({ value }) => {
      if (formMode === "edit") {
        if (!connection) return;
        const updated = buildConnectionFromForm(value, connection.id, {
          status: connection.status,
          latency: connection.latency,
          lastSync: connection.lastSync,
          errorMessage: connection.errorMessage,
          lastActivityAt: connection.lastActivityAt,
        });
        await updateConnection(updated);
      } else {
        const created = buildConnectionFromForm(value, crypto.randomUUID(), {
          status: "Disconnected",
          latency: "--",
          lastSync: "Never",
        });
        await addConnection(created);
      }
      onSaved?.();
    },
    validators: {
      onChange: connectionSchema.superRefine((value, ctx) => {
        const issues = validateConnection(
          connectionFormPolicy(value.engine),
          value,
          formMode,
        );
        for (const issue of issues) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: issue.message,
            path: [issue.path],
          });
        }
      }),
    },
  });

  // Edit mode hydrates a different connection if the prop changes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: form ref is stable
  useEffect(() => {
    if (formMode === "edit" && connection) {
      const next = defaultValuesFromConnection(connection);
      form.reset(next);
      setSelectedEngine(next.engine);
    }
  }, [connection, formMode]);

  const handleEngineChange = (engine: DatabaseEngine) => {
    if (formMode === "edit") return; // picker is disabled in edit mode
    form.setFieldValue("engine", engine);
    setSelectedEngine(engine);
    // Engine-specific fields reset to the new engine's neutral
    // defaults. Common fields (name/host/port/user/password/role)
    // survive the switch — the user keeps what they typed.
    switch (engine) {
      case "PostgreSQL":
      case "MySQL":
        form.setFieldValue("ssl", true);
        break;
      case "SQLite":
        // No engine-specific fields. SQLite's `database` field is
        // a file path; the user keeps whatever they typed.
        break;
      case "ClickHouse":
        form.setFieldValue("useHttps", false);
        form.setFieldValue("urlPath", "");
        break;
      case "Redis":
        form.setFieldValue("dbNumber", 0);
        form.setFieldValue("useTls", false);
        form.setFieldValue("verifyTlsCert", true);
        break;
    }
  };

  const handleCancel = () => {
    if (formMode === "new") {
      form.reset(EMPTY_NEW_DEFAULTS);
      setSelectedEngine("PostgreSQL");
    }
    setShowPassword(false);
    setTestStatus({ state: "idle" });
    onCancel?.();
  };

  const isSQLite = policy.kind === "file";
  const isClickHouse = policy.kind === "clickhouse-http";
  const isRedis = policy.kind === "redis";
  const showSslToggle = policy.kind === "host-auth" && policy.showSslToggle;

  const portPlaceholderForCh = (useHttps: boolean) =>
    policy.kind === "clickhouse-http"
      ? useHttps
        ? String(policy.defaultPortHttps)
        : String(policy.defaultPortHttp)
      : "";
  const portPlaceholder =
    policy.kind === "host-auth"
      ? String(policy.defaultPort)
      : policy.kind === "redis"
        ? String(policy.defaultPort)
        : policy.kind === "clickhouse-http"
          ? // Switches based on the form's live `useHttps`; the
            // form.Subscribe below renders the CH advanced section
            // separately, so this string is only used when CH renders
            // the host/port row above. We pick http-port by default
            // and the toggle re-renders the field below.
            portPlaceholderForCh(form.state.values.useHttps ?? false)
          : "";
  const databasePlaceholder = isSQLite ? "/path/to/db.sqlite" : "core";
  const databaseLabel = isSQLite ? "Database file" : "Database";

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
                onValueChange={(value) =>
                  handleEngineChange(value as DatabaseEngine)
                }
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
                  max={policy.kind === "redis" ? policy.maxDbNumber : 15}
                  placeholder={
                    policy.kind === "redis"
                      ? String(policy.defaultDbNumber)
                      : "0"
                  }
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
                    {isRedis
                      ? "Password (optional)"
                      : formMode === "edit"
                        ? "Password (leave blank to keep existing)"
                        : "Password"}
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

            {showSslToggle ? (
              <form.Field name="ssl">
                {(field) => (
                  <label
                    htmlFor="connection-ssl"
                    className="flex items-center justify-between rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2.5"
                  >
                    <span className="flex flex-col gap-0.5">
                      <span className="text-xs font-medium">SSL</span>
                      <span className="text-[0.6875rem] text-text-muted">
                        Negotiate TLS on the wire protocol.
                      </span>
                    </span>
                    <Switch
                      id="connection-ssl"
                      checked={field.state.value ?? true}
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
                            onCheckedChange={(value) =>
                              field.handleChange(value)
                            }
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
          {formMode === "new" ? (
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
          ) : null}
          <Button
            type="submit"
            size="sm"
            className="flex-1"
            disabled={form.state.isSubmitting || !form.state.isValid}
          >
            {form.state.isSubmitting
              ? "Saving…"
              : formMode === "edit"
                ? "Save changes"
                : "Create Connection"}
          </Button>
        </div>
        {formMode === "new" ? (
          <div className="flex items-center gap-1.5 text-[0.6875rem] text-text-muted">
            <IconShieldLock className="size-3 text-accent-green" />
            {credentialMode === "plain-sqlite"
              ? "Credentials are stored in the local SQLite database without encryption."
              : credentialMode === "keychain"
                ? "Credentials are stored with the OS keychain."
                : "Credentials are encrypted in the local SQLite database."}
          </div>
        ) : null}
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

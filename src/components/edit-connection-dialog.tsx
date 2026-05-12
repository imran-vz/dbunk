import { useForm } from "@tanstack/react-form";
import { useEffect, useState } from "react";
import * as z from "zod";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
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
import { type Connection, type DatabaseEngine, useAppStore } from "@/lib/store";

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
    useHttps: z.boolean().optional(),
    urlPath: z.string().optional(),
    dbNumber: z.number().int().min(0).max(15).optional(),
    useTls: z.boolean().optional(),
    verifyTlsCert: z.boolean().optional(),
  })
  .superRefine((value, ctx) => {
    if (value.engine === "SQLite") {
      if (!value.database?.trim()) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: "Database file is required",
          path: ["database"],
        });
      }
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
    if (value.engine === "Redis") {
      return;
    }
    if (!value.database?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "Database is required",
        path: ["database"],
      });
    }
    if (!value.user?.trim()) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: "User is required",
        path: ["user"],
      });
    }
  });

type ConnectionFormData = z.infer<typeof connectionSchema>;

interface EditConnectionDialogProps {
  connection: Connection | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function EditConnectionDialog({
  connection,
  open,
  onOpenChange,
}: EditConnectionDialogProps) {
  const [selectedEngine, setSelectedEngine] =
    useState<DatabaseEngine>("PostgreSQL");
  const [useHttps, setUseHttps] = useState(false);
  const updateConnection = useAppStore((state) => state.updateConnection);

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
      engine: "PostgreSQL" as const,
      host: "",
      database: "",
      port: 5432,
      user: "",
      password: "",
      role: "read/write",
      useHttps: false,
      urlPath: "",
      dbNumber: 0,
      useTls: false,
      verifyTlsCert: true,
    } as ConnectionFormData,
    onSubmit: async ({ value }) => {
      if (!connection) return;
      await updateConnection({
        ...connection,
        name: value.name,
        database: value.database ?? "",
        engine: value.engine,
        host: value.host ?? "",
        port: value.port ?? 0,
        user: value.user ?? "",
        password: value.password ?? "",
        role: value.role || "read/write",
        useHttps:
          value.engine === "ClickHouse" ? (value.useHttps ?? false) : false,
        urlPath: value.engine === "ClickHouse" ? (value.urlPath ?? "") : "",
        dbNumber: value.engine === "Redis" ? (value.dbNumber ?? 0) : 0,
        useTls: value.engine === "Redis" ? (value.useTls ?? false) : false,
        verifyTlsCert:
          value.engine === "Redis" ? (value.verifyTlsCert ?? true) : true,
      });
      onOpenChange(false);
    },
    validators: {
      onChange: connectionSchema,
    },
  });

  // Reset form when connection changes
  useEffect(() => {
    if (connection && open) {
      form.reset();
      form.setFieldValue("name", connection.name);
      form.setFieldValue("engine", connection.engine);
      form.setFieldValue("host", connection.host);
      form.setFieldValue("database", connection.database);
      form.setFieldValue("port", connection.port || 5432);
      form.setFieldValue("user", connection.user);
      form.setFieldValue("password", connection.password);
      form.setFieldValue("role", connection.role);
      form.setFieldValue("useHttps", connection.useHttps ?? false);
      form.setFieldValue("urlPath", connection.urlPath ?? "");
      form.setFieldValue("dbNumber", connection.dbNumber ?? 0);
      form.setFieldValue("useTls", connection.useTls ?? false);
      form.setFieldValue("verifyTlsCert", connection.verifyTlsCert ?? true);
      setSelectedEngine(connection.engine);
      setUseHttps(connection.useHttps ?? false);
    }
  }, [connection, open, form]);

  const handleOpenChange = (newOpen: boolean) => {
    onOpenChange(newOpen);
    if (!newOpen) {
      form.reset();
    }
  };

  return (
    <AlertDialog open={open} onOpenChange={handleOpenChange}>
      <AlertDialogContent>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            e.stopPropagation();
            form.handleSubmit();
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>Edit connection</AlertDialogTitle>
            <AlertDialogDescription>
              Update the connection details for{" "}
              {connection?.name ?? "this connection"}.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="grid gap-3 py-4">
            <form.Field name="name">
              {(field) => (
                <div className="grid gap-1">
                  <Label htmlFor="edit-connection-name">Connection name</Label>
                  <Input
                    id="edit-connection-name"
                    placeholder="Primary Postgres"
                    value={field.state.value}
                    onChange={(e) => field.handleChange(e.target.value)}
                    onBlur={field.handleBlur}
                  />
                  {field.state.meta.errors?.[0] && (
                    <p className="text-xs text-destructive">
                      {
                        (field.state.meta.errors[0] as { message?: string })
                          ?.message
                      }
                    </p>
                  )}
                </div>
              )}
            </form.Field>

            <div className="grid grid-cols-2 gap-2">
              <form.Field name="engine">
                {(field) => (
                  <div className="grid gap-1">
                    <Label htmlFor="edit-connection-engine">Engine</Label>
                    <Select
                      value={field.state.value}
                      onValueChange={(value) => {
                        const engine = value as DatabaseEngine;
                        field.handleChange(engine);
                        setSelectedEngine(engine);
                      }}
                    >
                      <SelectTrigger
                        id="edit-connection-engine"
                        className="w-full"
                      >
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
                    {field.state.meta.errors?.[0] && (
                      <p className="text-xs text-destructive">
                        {
                          (field.state.meta.errors[0] as { message?: string })
                            ?.message
                        }
                      </p>
                    )}
                  </div>
                )}
              </form.Field>

              {!isRedis ? (
                <form.Field name="database">
                  {(field) => (
                    <div className="grid gap-1">
                      <Label htmlFor="edit-connection-database">
                        {databaseLabel}
                      </Label>
                      <Input
                        id="edit-connection-database"
                        placeholder={databasePlaceholder}
                        value={field.state.value ?? ""}
                        onChange={(e) => field.handleChange(e.target.value)}
                        onBlur={field.handleBlur}
                      />
                      {field.state.meta.errors?.[0] && (
                        <p className="text-xs text-destructive">
                          {
                            (field.state.meta.errors[0] as { message?: string })
                              ?.message
                          }
                        </p>
                      )}
                    </div>
                  )}
                </form.Field>
              ) : (
                <form.Field name="dbNumber">
                  {(field) => (
                    <div className="grid gap-1">
                      <Label htmlFor="edit-connection-db-number">
                        DB number
                      </Label>
                      <Input
                        id="edit-connection-db-number"
                        type="number"
                        min={0}
                        max={15}
                        placeholder="0"
                        value={field.state.value ?? 0}
                        onChange={(e) =>
                          field.handleChange(Number(e.target.value))
                        }
                        onBlur={field.handleBlur}
                      />
                      {field.state.meta.errors?.[0] && (
                        <p className="text-xs text-destructive">
                          {
                            (field.state.meta.errors[0] as { message?: string })
                              ?.message
                          }
                        </p>
                      )}
                    </div>
                  )}
                </form.Field>
              )}
            </div>

            {!isSQLite && (
              <>
                <div className="grid grid-cols-2 gap-2">
                  <form.Field name="host">
                    {(field) => (
                      <div className="grid gap-1">
                        <Label htmlFor="edit-connection-host">Host</Label>
                        <Input
                          id="edit-connection-host"
                          placeholder="localhost"
                          value={field.state.value ?? ""}
                          onChange={(e) => field.handleChange(e.target.value)}
                          onBlur={field.handleBlur}
                        />
                        {field.state.meta.errors?.[0] && (
                          <p className="text-xs text-destructive">
                            {
                              (
                                field.state.meta.errors[0] as {
                                  message?: string;
                                }
                              )?.message
                            }
                          </p>
                        )}
                      </div>
                    )}
                  </form.Field>

                  <form.Field name="port">
                    {(field) => (
                      <div className="grid gap-1">
                        <Label htmlFor="edit-connection-port">Port</Label>
                        <Input
                          id="edit-connection-port"
                          type="number"
                          placeholder={portPlaceholder}
                          value={field.state.value ?? ""}
                          onChange={(e) =>
                            field.handleChange(Number(e.target.value))
                          }
                          onBlur={field.handleBlur}
                        />
                        {field.state.meta.errors?.[0] && (
                          <p className="text-xs text-destructive">
                            {
                              (
                                field.state.meta.errors[0] as {
                                  message?: string;
                                }
                              )?.message
                            }
                          </p>
                        )}
                      </div>
                    )}
                  </form.Field>
                </div>

                <div className="grid grid-cols-2 gap-2">
                  <form.Field name="user">
                    {(field) => (
                      <div className="grid gap-1">
                        <Label htmlFor="edit-connection-user">User</Label>
                        <Input
                          id="edit-connection-user"
                          placeholder="postgres"
                          value={field.state.value ?? ""}
                          onChange={(e) => field.handleChange(e.target.value)}
                          onBlur={field.handleBlur}
                        />
                        {field.state.meta.errors?.[0] && (
                          <p className="text-xs text-destructive">
                            {
                              (
                                field.state.meta.errors[0] as {
                                  message?: string;
                                }
                              )?.message
                            }
                          </p>
                        )}
                      </div>
                    )}
                  </form.Field>

                  <form.Field name="password">
                    {(field) => (
                      <div className="grid gap-1">
                        <Label htmlFor="edit-connection-password">
                          Password
                        </Label>
                        <Input
                          id="edit-connection-password"
                          type="password"
                          placeholder="Leave blank to keep existing password"
                          value={field.state.value ?? ""}
                          onChange={(e) => field.handleChange(e.target.value)}
                          onBlur={field.handleBlur}
                        />
                        {field.state.meta.errors?.[0] && (
                          <p className="text-xs text-destructive">
                            {
                              (
                                field.state.meta.errors[0] as {
                                  message?: string;
                                }
                              )?.message
                            }
                          </p>
                        )}
                      </div>
                    )}
                  </form.Field>
                </div>

                {isClickHouse && (
                  <>
                    <form.Field name="useHttps">
                      {(field) => (
                        <label
                          htmlFor="edit-connection-use-https"
                          className="flex items-center justify-between rounded-md border px-3 py-2.5"
                        >
                          <span className="flex flex-col gap-0.5">
                            <span className="text-xs font-medium">
                              Use HTTPS
                            </span>
                            <span className="text-xs text-muted-foreground">
                              Connect to ClickHouse over TLS (port 8443).
                            </span>
                          </span>
                          <Switch
                            id="edit-connection-use-https"
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
                        <div className="grid gap-1">
                          <Label htmlFor="edit-connection-url-path">
                            URL path
                          </Label>
                          <Input
                            id="edit-connection-url-path"
                            placeholder="/clickhouse (optional)"
                            value={field.state.value ?? ""}
                            onChange={(e) => field.handleChange(e.target.value)}
                            onBlur={field.handleBlur}
                          />
                        </div>
                      )}
                    </form.Field>
                  </>
                )}
                {isRedis && (
                  <>
                    <form.Field name="useTls">
                      {(field) => (
                        <label
                          htmlFor="edit-connection-use-tls"
                          className="flex items-center justify-between rounded-md border px-3 py-2.5"
                        >
                          <span className="flex flex-col gap-0.5">
                            <span className="text-xs font-medium">Use TLS</span>
                            <span className="text-xs text-muted-foreground">
                              Connect over TLS (rediss://).
                            </span>
                          </span>
                          <Switch
                            id="edit-connection-use-tls"
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
                                htmlFor="edit-connection-verify-tls-cert"
                                className="flex items-center justify-between rounded-md border px-3 py-2.5"
                              >
                                <span className="flex flex-col gap-0.5">
                                  <span className="text-xs font-medium">
                                    Verify TLS certificate
                                  </span>
                                  <span className="text-xs text-muted-foreground">
                                    Disable for self-signed dev servers.
                                  </span>
                                </span>
                                <Switch
                                  id="edit-connection-verify-tls-cert"
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
                )}
              </>
            )}
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel type="button">Cancel</AlertDialogCancel>
            <Button
              type="submit"
              disabled={form.state.isSubmitting || !form.state.isValid}
            >
              {form.state.isSubmitting ? "Saving..." : "Save changes"}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  );
}

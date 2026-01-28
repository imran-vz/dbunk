import { IconPlus } from "@tabler/icons-react";
import { useForm } from "@tanstack/react-form";
import { useState } from "react";
import * as z from "zod";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button, buttonVariants } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { type DatabaseEngine, useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

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

interface NewConnectionDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function NewConnectionDialog({
  open,
  onOpenChange,
}: NewConnectionDialogProps) {
  const [selectedEngine, setSelectedEngine] =
    useState<DatabaseEngine>("PostgreSQL");
  const addConnection = useAppStore((state) => state.addConnection);

  const isSQLite = selectedEngine === "SQLite";
  const portPlaceholder =
    selectedEngine === "MySQL"
      ? "3306"
      : selectedEngine === "ClickHouse"
        ? "8123"
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
      });
      form.reset();
      setSelectedEngine("PostgreSQL");
      onOpenChange(false);
    },
    validators: {
      onChange: connectionSchema,
    },
  });

  const handleOpenChange = (newOpen: boolean) => {
    onOpenChange(newOpen);
    if (!newOpen) {
      form.reset();
      setSelectedEngine("PostgreSQL");
    }
  };

  return (
    <AlertDialog open={open} onOpenChange={handleOpenChange}>
      <AlertDialogTrigger
        aria-label="Add connection"
        className={cn(buttonVariants({ variant: "outline", size: "icon-xs" }))}
      >
        <IconPlus />
      </AlertDialogTrigger>
      <AlertDialogContent>
        <form
          onSubmit={(e) => {
            e.preventDefault();
            e.stopPropagation();
            form.handleSubmit();
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>New connection</AlertDialogTitle>
            <AlertDialogDescription>
              Save database credentials securely and reuse them across tabs.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <div className="grid gap-3 py-4">
            <form.Field name="name">
              {(field) => (
                <div className="grid gap-1">
                  <Label htmlFor="connection-name">Connection name</Label>
                  <Input
                    id="connection-name"
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
                    <Label htmlFor="connection-engine">Engine</Label>
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

              <form.Field name="database">
                {(field) => (
                  <div className="grid gap-1">
                    <Label htmlFor="connection-database">{databaseLabel}</Label>
                    <Input
                      id="connection-database"
                      placeholder={databasePlaceholder}
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
            </div>

            {!isSQLite && (
              <>
                <div className="grid grid-cols-2 gap-2">
                  <form.Field name="host">
                    {(field) => (
                      <div className="grid gap-1">
                        <Label htmlFor="connection-host">Host</Label>
                        <Input
                          id="connection-host"
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
                        <Label htmlFor="connection-port">Port</Label>
                        <Input
                          id="connection-port"
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
                        <Label htmlFor="connection-user">User</Label>
                        <Input
                          id="connection-user"
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
                        <Label htmlFor="connection-password">Password</Label>
                        <Input
                          id="connection-password"
                          type="password"
                          placeholder="••••••••"
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
              </>
            )}
          </div>
          <AlertDialogFooter>
            <AlertDialogCancel
              type="button"
              onClick={() => {
                form.reset();
                setSelectedEngine("PostgreSQL");
              }}
            >
              Cancel
            </AlertDialogCancel>
            <Button
              type="submit"
              disabled={form.state.isSubmitting || !form.state.isValid}
            >
              {form.state.isSubmitting ? "Saving..." : "Save connection"}
            </Button>
          </AlertDialogFooter>
        </form>
      </AlertDialogContent>
    </AlertDialog>
  );
}

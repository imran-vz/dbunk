import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ENVIRONMENT_META, resolveSafetyPolicy } from "@/lib/safety-policy";
import type { ConnectionEnvironment, SafeMode } from "@/lib/store";
import { cn } from "@/lib/utils";

import { ToggleSwitchRow } from "./field-helpers";
import type { ConnectionFormApi } from "./use-connection-form";

const environments: ConnectionEnvironment[] = [
  "development",
  "test",
  "staging",
  "production",
];

const safeModes: SafeMode[] = ["inherit", "disabled", "protected", "strict"];

const environmentDot = {
  development: "bg-text-muted",
  test: "bg-info",
  staging: "bg-warning",
  production: "bg-danger",
} satisfies Record<ConnectionEnvironment, string>;

function environmentFromValue(value: string | null): ConnectionEnvironment {
  return (
    environments.find((environment) => environment === value) ?? "development"
  );
}

function safeModeFromValue(value: string | null): SafeMode {
  return safeModes.find((mode) => mode === value) ?? "inherit";
}

export function EnvironmentField({ form }: { form: ConnectionFormApi }) {
  return (
    <form.Field name="environment">
      {(field) => {
        const environment = field.state.value ?? "development";
        const meta = ENVIRONMENT_META[environment];
        return (
          <div className="grid gap-1.5">
            <Label htmlFor="connection-environment">Environment</Label>
            <Select
              value={environment}
              onValueChange={(value) =>
                field.handleChange(environmentFromValue(value))
              }
            >
              <SelectTrigger id="connection-environment" className="w-full">
                <SelectValue>
                  <span
                    className={cn(
                      "size-1.5 rounded-full",
                      environmentDot[environment],
                    )}
                  />
                  {meta.label}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {environments.map((value) => (
                  <SelectItem key={value} value={value}>
                    <span
                      className={cn(
                        "size-1.5 rounded-full",
                        environmentDot[value],
                      )}
                    />
                    {ENVIRONMENT_META[value].label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <p className="m-0 text-2xs text-text-muted">{meta.description}</p>
          </div>
        );
      }}
    </form.Field>
  );
}

export function SafetyFields({
  form,
  showReadOnly = true,
}: {
  form: ConnectionFormApi;
  showReadOnly?: boolean;
}) {
  return (
    <div className="grid gap-3 border-l-2 border-border-strong pl-3">
      <form.Subscribe
        selector={(state) =>
          [state.values.environment, state.values.safeMode] as const
        }
      >
        {([environmentValue, safeModeValue]) => {
          const environment = environmentValue ?? "development";
          const safeMode = safeModeValue ?? "inherit";
          const resolved = resolveSafetyPolicy({ environment, safeMode });
          const inherited = resolveSafetyPolicy({
            environment,
            safeMode: "inherit",
          });
          return (
            <form.Field name="safeMode">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-safe-mode">Safe Mode</Label>
                  <Select
                    value={safeMode}
                    onValueChange={(value) =>
                      field.handleChange(safeModeFromValue(value))
                    }
                  >
                    <SelectTrigger id="connection-safe-mode" className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="inherit">
                        Inherit ({titleCase(inherited.level)})
                      </SelectItem>
                      <SelectItem value="disabled">Disabled</SelectItem>
                      <SelectItem value="protected">Protected</SelectItem>
                      <SelectItem value="strict">Strict</SelectItem>
                    </SelectContent>
                  </Select>
                  <p className="m-0 text-2xs text-text-muted">
                    Resolves to {titleCase(resolved.level)} for{" "}
                    {ENVIRONMENT_META[environment].label}.
                  </p>
                </div>
              )}
            </form.Field>
          );
        }}
      </form.Subscribe>
      {showReadOnly ? (
        <form.Field name="readOnly">
          {(field) => (
            <ToggleSwitchRow
              id="connection-read-only"
              title="Read-only"
              description="Block every relational write on this connection."
              checked={field.state.value ?? false}
              onCheckedChange={(value) => field.handleChange(value)}
            />
          )}
        </form.Field>
      ) : null}
    </div>
  );
}

function titleCase(value: string): string {
  return `${value.charAt(0).toUpperCase()}${value.slice(1)}`;
}

import { useEffect } from "react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useAppStore } from "@/lib/store";

import { FieldError, ToggleSwitchRow } from "./field-helpers";
import { FIELD_ERROR } from "./form-utils";
import type { ConnectionFormApi } from "./use-connection-form";

export function TunnelFields({ form }: { form: ConnectionFormApi }) {
  const bastions = useAppStore((state) => state.bastionServers);
  const bastionStatus = useAppStore((state) => state.bastionStatus);
  const loadBastionServers = useAppStore((state) => state.loadBastionServers);

  useEffect(() => {
    if (bastionStatus.state === "idle") {
      void loadBastionServers();
    }
  }, [bastionStatus.state, loadBastionServers]);

  return (
    <div className="grid gap-3 rounded-md border border-border-subtle bg-surface-panel p-3">
      <form.Field name="sshTunnelEnabled">
        {(field) => (
          <ToggleSwitchRow
            id="connection-ssh-tunnel-enabled"
            title="SSH tunnel"
            description="Route this connection through a saved Bastion Server."
            checked={field.state.value ?? false}
            onCheckedChange={(next) => field.handleChange(next)}
          />
        )}
      </form.Field>

      {form.state.values.sshTunnelEnabled ? (
        <>
          <form.Field name="sshTunnelBastionServerId">
            {(field) => (
              <div className="grid gap-1.5">
                <Label htmlFor="connection-ssh-tunnel-bastion">
                  Bastion Server
                </Label>
                <Select
                  value={field.state.value ?? ""}
                  onValueChange={(value) => field.handleChange(value ?? "")}
                  disabled={bastions.length === 0}
                >
                  <SelectTrigger
                    id="connection-ssh-tunnel-bastion"
                    className="w-full"
                  >
                    <SelectValue placeholder="Select Bastion Server" />
                  </SelectTrigger>
                  <SelectContent>
                    {bastions.map((bastion) => (
                      <SelectItem key={bastion.id} value={bastion.id}>
                        {bastion.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                {bastions.length === 0 ? (
                  <p className="text-[0.6875rem] text-text-muted">
                    Add a Bastion Server in Settings before enabling a tunnel.
                  </p>
                ) : null}
                <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
              </div>
            )}
          </form.Field>

          <div className="grid gap-3 sm:grid-cols-[1fr_7rem]">
            <form.Field name="sshTunnelLocalBindHost">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-ssh-tunnel-bind-host">
                    Local bind host
                  </Label>
                  <Input
                    id="connection-ssh-tunnel-bind-host"
                    placeholder="127.0.0.1"
                    value={field.state.value ?? ""}
                    onChange={(event) => field.handleChange(event.target.value)}
                    onBlur={field.handleBlur}
                  />
                  <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
                </div>
              )}
            </form.Field>

            <form.Field name="sshTunnelLocalPort">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-ssh-tunnel-local-port">
                    Local port
                  </Label>
                  <Input
                    id="connection-ssh-tunnel-local-port"
                    type="number"
                    placeholder="auto"
                    value={field.state.value ?? ""}
                    onChange={(event) =>
                      field.handleChange(parseOptionalPort(event.target.value))
                    }
                    onBlur={field.handleBlur}
                  />
                  <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
                </div>
              )}
            </form.Field>
          </div>
        </>
      ) : null}
    </div>
  );
}

function parseOptionalPort(value: string): number | undefined {
  if (!value.trim()) {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

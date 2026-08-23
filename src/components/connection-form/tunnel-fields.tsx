import { IconPlus, IconTrash } from "@tabler/icons-react";
import { useEffect } from "react";

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
import { Textarea } from "@/components/ui/textarea";
import type { BastionServer } from "@/lib/store";
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
                  <p className="text-2xs text-text-muted">
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

          <form.Field name="sshTunnelCompression">
            {(field) => (
              <ToggleSwitchRow
                id="connection-ssh-tunnel-compression"
                title="Compression"
                description="Request SSH compression for this tunnel."
                checked={field.state.value ?? false}
                onCheckedChange={(next) => field.handleChange(next)}
              />
            )}
          </form.Field>

          <div className="grid gap-3 sm:grid-cols-[1fr_auto]">
            <form.Field name="sshTunnelKeepaliveIntervalSeconds">
              {(field) => (
                <div className="grid gap-1.5">
                  <Label htmlFor="connection-ssh-tunnel-keepalive-interval">
                    Keepalive interval
                  </Label>
                  <Input
                    id="connection-ssh-tunnel-keepalive-interval"
                    type="number"
                    min={2}
                    max={3600}
                    placeholder="disabled"
                    value={field.state.value ?? ""}
                    onChange={(event) =>
                      field.handleChange(
                        parseOptionalNumber(event.target.value),
                      )
                    }
                    onBlur={field.handleBlur}
                  />
                  <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
                </div>
              )}
            </form.Field>

            <form.Field name="sshTunnelKeepaliveWantReply">
              {(field) => (
                <label
                  htmlFor="connection-ssh-tunnel-keepalive-reply"
                  className="flex h-full min-h-14 items-center justify-between gap-3 rounded-md border border-border-subtle bg-surface-panel-elevated px-3 py-2"
                >
                  <span className="grid gap-0.5">
                    <span className="text-xs font-medium">Require reply</span>
                    <span className="text-2xs text-text-muted">
                      Treat missing keepalive replies as SSH failure.
                    </span>
                  </span>
                  <Switch
                    id="connection-ssh-tunnel-keepalive-reply"
                    checked={field.state.value ?? true}
                    onCheckedChange={(next) => field.handleChange(next)}
                  />
                </label>
              )}
            </form.Field>
          </div>

          <form.Field name="sshTunnelJumpChain">
            {(field) => (
              <JumpChainField
                bastions={bastions}
                finalBastionId={
                  form.state.values.sshTunnelBastionServerId ?? ""
                }
                value={field.state.value ?? []}
                onChange={(value) => field.handleChange(value)}
                errorText={FIELD_ERROR(field.state.meta.errors)}
              />
            )}
          </form.Field>

          <form.Field name="sshTunnelProxyCommand">
            {(field) => (
              <div className="grid gap-1.5">
                <Label htmlFor="connection-ssh-tunnel-proxy-command">
                  Proxy command
                </Label>
                <Textarea
                  id="connection-ssh-tunnel-proxy-command"
                  className="min-h-16 font-mono"
                  placeholder="ssh -W %h:%p edge-gateway"
                  value={field.state.value ?? ""}
                  onChange={(event) => field.handleChange(event.target.value)}
                  onBlur={field.handleBlur}
                />
                <p className="text-2xs text-text-muted">
                  Applied to the first SSH hop. Use %h and %p for the host and
                  port.
                </p>
                <FieldError text={FIELD_ERROR(field.state.meta.errors)} />
              </div>
            )}
          </form.Field>
        </>
      ) : null}
    </div>
  );
}

function parseOptionalPort(value: string): number | undefined {
  return parseOptionalNumber(value);
}

function parseOptionalNumber(value: string): number | undefined {
  if (!value.trim()) {
    return undefined;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : undefined;
}

/* oxlint-disable react/no-array-index-key -- Duplicate and empty draft hops are distinguished by their ordered position. */
export function JumpChainField({
  bastions,
  finalBastionId,
  value,
  onChange,
  errorText,
}: {
  bastions: BastionServer[];
  finalBastionId: string;
  value: string[];
  onChange: (value: string[]) => void;
  errorText: string | null;
}) {
  const chain = value;
  const addHop = () => onChange([...chain, ""]);
  const updateHop = (index: number, bastionId: string) => {
    onChange(
      chain.map((current, idx) => (idx === index ? bastionId : current)),
    );
  };
  const removeHop = (index: number) => {
    onChange(chain.filter((_, idx) => idx !== index));
  };

  return (
    <div className="grid gap-2 rounded-md border border-border-subtle bg-surface-panel-elevated p-3">
      <div className="flex items-center justify-between gap-3">
        <div>
          <Label>Jump chain</Label>
          <p className="mt-0.5 text-2xs text-text-muted">
            Optional intermediate Bastion Servers before the selected one.
          </p>
        </div>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={bastions.length === 0}
          onClick={addHop}
        >
          <IconPlus className="size-3" />
          Add hop
        </Button>
      </div>
      {chain.length === 0 ? (
        <div className="rounded-sm border border-dashed border-border-subtle px-3 py-2 text-2xs text-text-muted">
          No jump hops configured.
        </div>
      ) : (
        <div className="grid gap-2">
          {chain.map((bastionId, index) => (
            // oxlint-disable-next-line react/no-array-index-key -- Duplicate and empty draft hops are distinguished by their ordered position.
            <div
              key={`${index}-${bastionId || "empty"}`}
              className="grid gap-2 sm:grid-cols-[1.5rem_1fr_auto]"
            >
              <div className="flex items-center text-2xs text-text-muted">
                {index + 1}
              </div>
              <Select
                value={bastionId}
                onValueChange={(next) => updateHop(index, next ?? "")}
              >
                <SelectTrigger className="w-full">
                  <SelectValue placeholder="Select Bastion Server" />
                </SelectTrigger>
                <SelectContent>
                  {jumpOptions(bastions, finalBastionId, bastionId).map(
                    (bastion) => (
                      <SelectItem key={bastion.id} value={bastion.id}>
                        {bastion.name}
                      </SelectItem>
                    ),
                  )}
                </SelectContent>
              </Select>
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                aria-label={`Remove jump hop ${index + 1}`}
                onClick={() => removeHop(index)}
              >
                <IconTrash className="size-3" />
              </Button>
            </div>
          ))}
        </div>
      )}
      <FieldError text={errorText} />
    </div>
  );
}

/* oxlint-enable react/no-array-index-key */
function jumpOptions(
  bastions: BastionServer[],
  finalBastionId: string,
  currentId: string,
): BastionServer[] {
  return bastions.filter(
    (bastion) => bastion.id !== finalBastionId || bastion.id === currentId,
  );
}

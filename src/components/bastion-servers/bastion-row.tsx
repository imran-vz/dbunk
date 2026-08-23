import { IconPlugConnected, IconRefresh, IconTrash } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { BastionServer } from "@/lib/store";
import { cn } from "@/lib/utils";

import { authLabel } from "./helpers";
import type { HostKeyResetState, TestState } from "./types";

export function BastionRow({
  bastion,
  active,
  referenceCount,
  hostKeyReset,
  testState,
  onEdit,
  onTest,
  onRequestHostKeyReset,
  onHostKeyResetInput,
  onConfirmHostKeyReset,
  onCancelHostKeyReset,
  onDelete,
}: {
  bastion: BastionServer;
  active: boolean;
  referenceCount: number;
  hostKeyReset: HostKeyResetState;
  testState: TestState;
  onEdit: () => void;
  onTest: () => void;
  onRequestHostKeyReset: () => void;
  onHostKeyResetInput: (value: string) => void;
  onConfirmHostKeyReset: () => void;
  onCancelHostKeyReset: () => void;
  onDelete: () => void;
}) {
  const isTesting =
    testState.state === "running" && testState.id === bastion.id;
  const latestTest =
    testState.state !== "idle" && testState.id === bastion.id
      ? testState
      : null;

  return (
    <div
      className={cn(
        "grid gap-3 px-4 py-3 transition-colors",
        active ? "bg-accent/10" : "hover:bg-surface-panel",
      )}
    >
      <div className="grid gap-3 md:grid-cols-[minmax(0,1fr)_auto]">
        <button type="button" className="text-left" onClick={onEdit}>
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium text-foreground">
              {bastion.name}
            </span>
            <span className="rounded-sm bg-surface-panel-elevated px-1.5 py-0.5 text-2xs text-text-muted">
              {authLabel(bastion.authMethod)}
            </span>
            <span className="rounded-sm bg-surface-panel-elevated px-1.5 py-0.5 text-2xs text-text-muted">
              {referenceCount === 1
                ? "1 Connection"
                : `${referenceCount} Connections`}
            </span>
          </div>
          <div className="mt-1 text-xs text-text-secondary">
            {bastion.user}@{bastion.host}:{bastion.port}
          </div>
          <div
            className={cn(
              "mt-1 text-2xs",
              bastion.hostKeyFingerprint ? "text-text-muted" : "text-warning",
            )}
          >
            Host key: {bastion.hostKeyFingerprint ?? "not trusted yet"}
          </div>
          {latestTest?.state === "success" ? (
            <div className="mt-1 text-2xs text-accent">
              SSH reachable in {latestTest.latencyMs}ms
            </div>
          ) : null}
          {latestTest?.state === "error" ? (
            <div className="mt-1 text-2xs text-danger">{latestTest.error}</div>
          ) : null}
        </button>
        <div className="flex flex-wrap items-start justify-end gap-2">
          <Button
            type="button"
            variant="outline"
            disabled={isTesting}
            onClick={onTest}
          >
            <IconPlugConnected className="size-3.5" />
            {isTesting ? "Testing" : "Test"}
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={onRequestHostKeyReset}
          >
            <IconRefresh className="size-3.5" />
            Reset trust
          </Button>
          <Button type="button" variant="destructive" onClick={onDelete}>
            <IconTrash className="size-3.5" />
            Delete
          </Button>
        </div>
      </div>
      {hostKeyReset ? (
        <fieldset
          aria-label={`Confirm host-key reset for ${bastion.name}`}
          className="grid gap-2 rounded-md border border-warning/30 bg-warning/10 p-3"
        >
          <legend className="text-xs font-medium text-foreground">
            Reset trusted host key
          </legend>
          <p className="text-2xs text-text-secondary">
            Current fingerprint:{" "}
            {bastion.hostKeyFingerprint ?? "not trusted yet"}. Type{" "}
            {bastion.host} to reset trust for the next SSH handshake.
          </p>
          <div className="grid gap-2 sm:grid-cols-[1fr_auto_auto]">
            <Input
              aria-label={`Type ${bastion.host} to confirm host-key reset`}
              value={hostKeyReset.confirmHost}
              onChange={(event) => onHostKeyResetInput(event.target.value)}
            />
            <Button
              type="button"
              variant="destructive"
              disabled={hostKeyReset.confirmHost !== bastion.host}
              onClick={onConfirmHostKeyReset}
            >
              Reset trust
            </Button>
            <Button
              type="button"
              variant="outline"
              onClick={onCancelHostKeyReset}
            >
              Cancel
            </Button>
          </div>
        </fieldset>
      ) : null}
    </div>
  );
}

import { IconKey } from "@tabler/icons-react";

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
import { Textarea } from "@/components/ui/textarea";
import type { BastionAuthMethod, BastionServer } from "@/lib/store";
import { cn } from "@/lib/utils";

import { analyzePrivateKeyContent, secretSummary } from "./helpers";
import type { BastionDraft } from "./types";

export function BastionForm({
  draft,
  editing,
  onChange,
  onSave,
  onCancel,
}: {
  draft: BastionDraft;
  editing: BastionServer | null;
  onChange: (draft: BastionDraft) => void;
  onSave: () => void;
  onCancel: () => void;
}) {
  const update = <K extends keyof BastionDraft>(
    key: K,
    value: BastionDraft[K],
  ) => onChange({ ...draft, [key]: value });

  const isEditing = editing !== null;
  const title = isEditing ? "Edit Bastion Server" : "New Bastion Server";

  return (
    <section className="rounded-lg border border-border-subtle bg-surface-window p-4">
      <div className="mb-4">
        <div className="flex items-center justify-between gap-3">
          <h2 className="text-sm font-semibold text-foreground">{title}</h2>
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={() => update("authMethod", "privateKeyContent")}
          >
            <IconKey />
            Capture key
          </Button>
        </div>
        <p className="mt-1 text-xs text-text-muted">
          Secrets are write-only. Blank secret fields keep existing values.
        </p>
      </div>
      <form
        className="grid gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          void onSave();
        }}
      >
        <TextField
          id="bastion-name"
          label="Name"
          value={draft.name}
          placeholder="Production bastion"
          onChange={(value) => update("name", value)}
        />
        <div className="grid gap-3 sm:grid-cols-[1fr_6rem] lg:grid-cols-1 xl:grid-cols-[1fr_6rem]">
          <TextField
            id="bastion-host"
            label="Host"
            value={draft.host}
            placeholder="bastion.example.com"
            onChange={(value) => update("host", value)}
          />
          <TextField
            id="bastion-port"
            label="Port"
            type="number"
            value={String(draft.port || "")}
            placeholder="22"
            onChange={(value) => update("port", Number(value))}
          />
        </div>
        <TextField
          id="bastion-user"
          label="Username"
          value={draft.user}
          placeholder="ubuntu"
          onChange={(value) => update("user", value)}
        />

        <div className="grid gap-1.5">
          <Label htmlFor="bastion-auth-method">Authentication</Label>
          <Select
            value={draft.authMethod}
            onValueChange={(value) =>
              // SAFETY: The value is constrained by the typed component or library contract at this boundary.
              update("authMethod", value as BastionAuthMethod)
            }
          >
            <SelectTrigger id="bastion-auth-method" className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="password">Password</SelectItem>
              <SelectItem value="privateKeyPath">Private key path</SelectItem>
              <SelectItem value="privateKeyContent">
                Private key content
              </SelectItem>
            </SelectContent>
          </Select>
        </div>

        {draft.authMethod === "password" ? (
          <TextField
            id="bastion-password"
            label={isEditing ? "Password (leave blank to keep)" : "Password"}
            type="password"
            value={draft.password ?? ""}
            placeholder="••••••••"
            onChange={(value) => update("password", value)}
          />
        ) : null}

        {draft.authMethod === "privateKeyPath" ? (
          <TextField
            id="bastion-private-key-path"
            label="Private key path"
            value={draft.privateKeyPath ?? ""}
            placeholder="/Users/me/.ssh/id_ed25519"
            onChange={(value) => update("privateKeyPath", value)}
          />
        ) : null}

        {draft.authMethod === "privateKeyContent" ? (
          <GuidedKeyCapture
            value={draft.privateKeyContent ?? ""}
            isEditing={isEditing}
            onChange={(value) => update("privateKeyContent", value)}
          />
        ) : null}

        {draft.authMethod !== "password" ? (
          <TextField
            id="bastion-passphrase"
            label="Passphrase (optional)"
            type="password"
            value={draft.passphrase ?? ""}
            placeholder="Leave blank if not needed"
            onChange={(value) =>
              onChange({ ...draft, passphrase: value, clearPassphrase: false })
            }
          />
        ) : null}

        {editing?.hasPassphrase && draft.authMethod !== "password" ? (
          <label className="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-panel px-3 py-2 text-xs text-text-secondary">
            <input
              type="checkbox"
              checked={draft.clearPassphrase}
              onChange={(event) =>
                onChange({
                  ...draft,
                  clearPassphrase: event.target.checked,
                  passphrase: event.target.checked ? "" : draft.passphrase,
                })
              }
            />
            Clear stored passphrase
          </label>
        ) : null}

        {editing ? (
          <div className="rounded-md border border-border-subtle bg-surface-panel px-3 py-2 text-2xs text-text-muted">
            Stored secrets: {secretSummary(editing)}
          </div>
        ) : null}

        <div className="mt-1 flex justify-end gap-2">
          <Button type="button" variant="outline" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="submit">{isEditing ? "Save changes" : "Save"}</Button>
        </div>
      </form>
    </section>
  );
}

function TextField({
  id,
  label,
  value,
  onChange,
  placeholder,
  type = "text",
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  type?: string;
}) {
  return (
    <div className="grid gap-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Input
        id={id}
        type={type}
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
    </div>
  );
}

function GuidedKeyCapture({
  value,
  isEditing,
  onChange,
}: {
  value: string;
  isEditing: boolean;
  onChange: (value: string) => void;
}) {
  const analysis = analyzePrivateKeyContent(value);
  return (
    <div className="grid gap-1.5">
      <Label htmlFor="bastion-private-key-content">
        {isEditing
          ? "Private key content (leave blank to keep)"
          : "Private key content"}
      </Label>
      <Textarea
        id="bastion-private-key-content"
        className="min-h-32 font-mono"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
      <div
        className={cn(
          "rounded-md border px-3 py-2 text-2xs",
          analysis.tone === "ready" &&
            "border-accent/30 bg-accent/10 text-accent",
          analysis.tone === "warning" &&
            "border-warning/30 bg-warning/10 text-warning",
          analysis.tone === "neutral" &&
            "border-border-subtle bg-surface-panel text-text-muted",
        )}
      >
        {analysis.message}
      </div>
    </div>
  );
}

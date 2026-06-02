import {
  IconPlugConnected,
  IconPlus,
  IconRefresh,
  IconTrash,
} from "@tabler/icons-react";
import { useEffect, useMemo, useState } from "react";

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
import type {
  BastionAuthMethod,
  BastionServer,
  SaveBastionServerInput,
  SecretChange,
} from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

type TestState =
  | { state: "idle" }
  | { state: "running"; id: string }
  | { state: "success"; id: string; latencyMs: number }
  | { state: "error"; id: string; error: string };

type BastionDraft = Omit<
  SaveBastionServerInput,
  "password" | "privateKeyContent" | "passphrase"
> & {
  password: string;
  privateKeyContent: string;
  passphrase: string;
  clearPassphrase: boolean;
};

export function BastionServersTab() {
  const bastions = useAppStore((state) => state.bastionServers);
  const status = useAppStore((state) => state.bastionStatus);
  const loadBastionServers = useAppStore((state) => state.loadBastionServers);
  const saveBastionServer = useAppStore((state) => state.saveBastionServer);
  const deleteBastionServer = useAppStore((state) => state.deleteBastionServer);
  const resetBastionHostKey = useAppStore((state) => state.resetBastionHostKey);
  const testBastionServer = useAppStore((state) => state.testBastionServer);

  const [draft, setDraft] = useState<BastionDraft>(() => blankDraft());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [testState, setTestState] = useState<TestState>({ state: "idle" });

  useEffect(() => {
    if (status.state === "idle") {
      void loadBastionServers();
    }
  }, [loadBastionServers, status.state]);

  const editing = useMemo(
    () => bastions.find((bastion) => bastion.id === editingId) ?? null,
    [bastions, editingId],
  );

  const startNew = () => {
    setEditingId(null);
    setDraft(blankDraft());
  };

  const startEdit = (bastion: BastionServer) => {
    setEditingId(bastion.id);
    setDraft(draftFromBastion(bastion));
  };

  const handleSave = async () => {
    const ok = await saveBastionServer(normalizeDraft(draft));
    if (ok) {
      startNew();
    }
  };

  const handleDelete = async (bastion: BastionServer) => {
    if (
      window.confirm(
        `Delete Bastion Server "${bastion.name}"? Connections that reference it must be changed first.`,
      )
    ) {
      const ok = await deleteBastionServer(bastion.id);
      if (ok && editingId === bastion.id) {
        startNew();
      }
    }
  };

  const handleResetHostKey = async (bastion: BastionServer) => {
    if (
      window.confirm(
        `Reset trusted host key for "${bastion.name}"? The next connection will trust the key presented by the SSH server.`,
      )
    ) {
      void resetBastionHostKey(bastion.id);
    }
  };

  const handleTest = async (bastion: BastionServer) => {
    setTestState({ state: "running", id: bastion.id });
    const result = await testBastionServer(bastion.id);
    setTestState(
      result.ok
        ? { state: "success", id: bastion.id, latencyMs: result.latencyMs }
        : { state: "error", id: bastion.id, error: result.error },
    );
  };

  return (
    <>
      <SectionHeader
        title="Bastion Servers"
        subtitle="Reusable SSH bastions for database connections that need a local tunnel."
      />
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto grid max-w-5xl gap-5 lg:grid-cols-[minmax(0,1fr)_22rem]">
          <section className="min-h-0 rounded-lg border border-border-subtle bg-surface-window">
            <div className="flex items-center justify-between border-b border-border-subtle px-4 py-3">
              <div>
                <h2 className="text-sm font-semibold text-foreground">
                  Saved Bastion Servers
                </h2>
                <p className="mt-1 text-xs text-text-muted">
                  Deletion is blocked while a Connection references the server.
                </p>
              </div>
              <Button type="button" variant="outline" onClick={startNew}>
                <IconPlus className="size-3.5" />
                New
              </Button>
            </div>
            {bastions.length === 0 ? (
              <div className="px-4 py-8 text-center text-xs text-text-muted">
                No Bastion Servers saved.
              </div>
            ) : (
              <div className="divide-y divide-border-subtle">
                {bastions.map((bastion) => (
                  <BastionRow
                    key={bastion.id}
                    bastion={bastion}
                    active={bastion.id === editingId}
                    testState={testState}
                    onEdit={() => startEdit(bastion)}
                    onTest={() => void handleTest(bastion)}
                    onResetHostKey={() => handleResetHostKey(bastion)}
                    onDelete={() => handleDelete(bastion)}
                  />
                ))}
              </div>
            )}
          </section>

          <BastionForm
            draft={draft}
            editing={editing}
            onChange={setDraft}
            onSave={handleSave}
            onCancel={startNew}
          />
        </div>
        {status.state === "error" ? (
          <div className="mx-auto mt-4 max-w-5xl rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
            {status.error}
          </div>
        ) : null}
      </div>
    </>
  );
}

function BastionRow({
  bastion,
  active,
  testState,
  onEdit,
  onTest,
  onResetHostKey,
  onDelete,
}: {
  bastion: BastionServer;
  active: boolean;
  testState: TestState;
  onEdit: () => void;
  onTest: () => void;
  onResetHostKey: () => void;
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
        "grid gap-3 px-4 py-3 transition-colors md:grid-cols-[minmax(0,1fr)_auto]",
        active ? "bg-accent-green/10" : "hover:bg-surface-panel",
      )}
    >
      <button type="button" className="text-left" onClick={onEdit}>
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium text-foreground">
            {bastion.name}
          </span>
          <span className="rounded-sm bg-surface-panel-elevated px-1.5 py-0.5 text-[0.625rem] text-text-muted">
            {authLabel(bastion.authMethod)}
          </span>
        </div>
        <div className="mt-1 text-xs text-text-secondary">
          {bastion.user}@{bastion.host}:{bastion.port}
        </div>
        <div className="mt-1 text-[0.6875rem] text-text-muted">
          Host key: {bastion.hostKeyFingerprint ?? "not trusted yet"}
        </div>
        {latestTest?.state === "success" ? (
          <div className="mt-1 text-[0.6875rem] text-accent-green">
            SSH reachable in {latestTest.latencyMs}ms
          </div>
        ) : null}
        {latestTest?.state === "error" ? (
          <div className="mt-1 text-[0.6875rem] text-danger">
            {latestTest.error}
          </div>
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
        <Button type="button" variant="outline" onClick={onResetHostKey}>
          <IconRefresh className="size-3.5" />
          Reset key
        </Button>
        <Button type="button" variant="destructive" onClick={onDelete}>
          <IconTrash className="size-3.5" />
          Delete
        </Button>
      </div>
    </div>
  );
}

function BastionForm({
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
        <h2 className="text-sm font-semibold text-foreground">{title}</h2>
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
          <div className="grid gap-1.5">
            <Label htmlFor="bastion-private-key-content">
              {isEditing
                ? "Private key content (leave blank to keep)"
                : "Private key content"}
            </Label>
            <Textarea
              id="bastion-private-key-content"
              className="min-h-28 font-mono"
              value={draft.privateKeyContent ?? ""}
              onChange={(event) =>
                update("privateKeyContent", event.target.value)
              }
            />
          </div>
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
          <div className="rounded-md border border-border-subtle bg-surface-panel px-3 py-2 text-[0.6875rem] text-text-muted">
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

function SectionHeader({
  title,
  subtitle,
}: {
  title: string;
  subtitle: string;
}) {
  return (
    <header className="shrink-0 border-b border-border-subtle bg-surface-window px-6 py-4">
      <h1 className="text-xl font-semibold tracking-tight text-foreground">
        {title}
      </h1>
      <p className="mt-1 text-xs text-text-muted">{subtitle}</p>
    </header>
  );
}

function blankDraft(): BastionDraft {
  return {
    id: crypto.randomUUID(),
    name: "",
    host: "",
    port: 22,
    user: "",
    authMethod: "privateKeyPath",
    privateKeyPath: "",
    password: "",
    privateKeyContent: "",
    passphrase: "",
    clearPassphrase: false,
  };
}

function draftFromBastion(bastion: BastionServer): BastionDraft {
  return {
    id: bastion.id,
    name: bastion.name,
    host: bastion.host,
    port: bastion.port,
    user: bastion.user,
    authMethod: bastion.authMethod,
    privateKeyPath: bastion.privateKeyPath ?? "",
    password: "",
    privateKeyContent: "",
    passphrase: "",
    clearPassphrase: false,
  };
}

function normalizeDraft(draft: BastionDraft): SaveBastionServerInput {
  const optional = (value?: string) => {
    const trimmed = value?.trim();
    return trimmed ? trimmed : undefined;
  };
  return {
    id: draft.id,
    name: draft.name.trim(),
    host: draft.host.trim(),
    port: draft.port || 22,
    user: draft.user.trim(),
    authMethod: draft.authMethod,
    privateKeyPath: optional(draft.privateKeyPath),
    password: secretChangeFromInput(draft.password),
    privateKeyContent: secretChangeFromInput(draft.privateKeyContent),
    passphrase: draft.clearPassphrase
      ? { action: "clear" }
      : secretChangeFromInput(draft.passphrase),
  };
}

function secretChangeFromInput(value: string): SecretChange {
  return value.trim() ? { action: "set", value } : { action: "keep" };
}

function authLabel(method: BastionAuthMethod): string {
  switch (method) {
    case "password":
      return "Password";
    case "privateKeyPath":
      return "Key path";
    case "privateKeyContent":
      return "Key content";
  }
}

function secretSummary(bastion: BastionServer): string {
  const parts = [
    bastion.hasPassword ? "password" : null,
    bastion.hasPrivateKeyContent ? "private key content" : null,
    bastion.hasPassphrase ? "passphrase" : null,
  ].filter(Boolean);
  return parts.length > 0 ? parts.join(", ") : "none";
}

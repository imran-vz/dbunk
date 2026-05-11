import { IconDatabase, IconKey, IconShieldLock } from "@tabler/icons-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { CredentialStorageMode } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const OPTIONS: Array<{
  id: CredentialStorageMode;
  label: string;
  detail: string;
  icon: typeof IconShieldLock;
}> = [
  {
    id: "encrypted-sqlite",
    label: "Encrypted SQLite",
    detail:
      "Recommended. Passwords are encrypted in the local SQLite database.",
    icon: IconShieldLock,
  },
  {
    id: "keychain",
    label: "OS keychain",
    detail: "Uses the operating system credential store. May prompt on macOS.",
    icon: IconKey,
  },
  {
    id: "plain-sqlite",
    label: "Unencrypted SQLite",
    detail: "Stores database passwords plainly in dbunk.sqlite.",
    icon: IconDatabase,
  },
];

export function SettingsView() {
  const settings = useAppStore((state) => state.appSettings);
  const status = useAppStore((state) => state.credentialStorageStatus);
  const changeCredentialStorage = useAppStore(
    (state) => state.changeCredentialStorage,
  );
  const resetCredentialStorage = useAppStore(
    (state) => state.resetCredentialStorage,
  );

  const current = settings?.credentialStorageMode ?? "encrypted-sqlite";
  const [selected, setSelected] = useState<CredentialStorageMode>(current);
  const [password, setPassword] = useState("");
  const isChanging = selected !== current;
  const needsPassword = selected === "encrypted-sqlite" && isChanging;

  const handleApply = async () => {
    const selectedOption = OPTIONS.find((option) => option.id === selected);
    const warning =
      selected === "plain-sqlite"
        ? "Database passwords will be stored as plaintext in ~/.config/dbunk/dbunk.sqlite."
        : selected === "encrypted-sqlite"
          ? "If you forget this credential password, saved database passwords cannot be recovered."
          : "The OS keychain may ask for permission when dbunk reads saved credentials.";
    if (
      !window.confirm(
        `Change credential storage to ${selectedOption?.label ?? selected}?\n\n${warning}`,
      )
    ) {
      return;
    }
    await changeCredentialStorage({
      mode: selected,
      password: needsPassword ? password : undefined,
    });
    setPassword("");
  };

  return (
    <div className="flex h-full min-h-0 flex-1 flex-col bg-surface-app">
      <header className="border-b border-border-subtle bg-surface-window px-6 py-4">
        <h1 className="text-xl font-semibold tracking-tight text-foreground">
          Settings
        </h1>
        <p className="mt-1 text-xs text-text-muted">
          System-wide dbunk preferences and local storage configuration.
        </p>
      </header>

      <div className="min-h-0 flex-1 overflow-auto p-6">
        <section className="mx-auto max-w-3xl rounded-lg border border-border-subtle bg-surface-window p-5">
          <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border-subtle pb-4">
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                Credential storage
              </h2>
              <p className="mt-1 text-xs leading-5 text-text-muted">
                Current mode: {labelFor(current)}. Settings are stored in{" "}
                {settings?.configDir ?? "~/.config/dbunk"}.
              </p>
            </div>
            <span
              className={cn(
                "rounded-md px-2 py-1 text-[0.6875rem]",
                settings?.credentialState === "ready"
                  ? "bg-accent-green/10 text-accent-green"
                  : "bg-warning/10 text-warning",
              )}
            >
              {settings?.credentialState === "ready" ? "Ready" : "Locked"}
            </span>
          </div>

          <div className="mt-4 grid gap-3">
            {OPTIONS.map((option) => {
              const Icon = option.icon;
              const active = selected === option.id;
              return (
                <button
                  key={option.id}
                  type="button"
                  onClick={() => setSelected(option.id)}
                  className={cn(
                    "flex items-start gap-3 rounded-lg border p-4 text-left transition-colors",
                    active
                      ? "border-accent-green/50 bg-accent-green/10"
                      : "border-border-subtle bg-surface-panel hover:border-border-strong",
                  )}
                >
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border-subtle bg-surface-window text-text-secondary">
                    <Icon className="size-4" />
                  </span>
                  <span>
                    <span className="block text-sm font-semibold">
                      {option.label}
                    </span>
                    <span className="mt-1 block text-xs leading-5 text-text-muted">
                      {option.detail}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>

          {needsPassword ? (
            <div className="mt-4 grid gap-2 rounded-lg border border-border-subtle bg-surface-panel p-4">
              <Input
                type="password"
                placeholder="New credential password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
              <div className="text-xs leading-5 text-text-muted">
                This password is not recoverable. Resetting credential storage
                deletes saved database passwords.
              </div>
            </div>
          ) : null}

          {status.state === "error" ? (
            <div className="mt-4 rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
              {status.error}
            </div>
          ) : null}

          <div className="mt-5 flex flex-wrap items-center justify-between gap-3">
            <Button
              type="button"
              variant="outline"
              onClick={() => {
                if (
                  window.confirm(
                    "Reset credential storage? Saved database passwords will be deleted and cannot be recovered.",
                  )
                ) {
                  void resetCredentialStorage();
                }
              }}
            >
              Reset credential storage
            </Button>
            <Button
              type="button"
              disabled={
                !isChanging ||
                status.state === "running" ||
                (needsPassword && !password)
              }
              onClick={handleApply}
            >
              {status.state === "running" ? "Applying..." : "Apply change"}
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
}

function labelFor(mode: CredentialStorageMode): string {
  return OPTIONS.find((option) => option.id === mode)?.label ?? mode;
}

import {
  IconDatabase,
  IconKey,
  IconLock,
  IconShieldLock,
} from "@tabler/icons-react";
import { useCallback, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { CredentialStorageMode } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

/**
 * Loads the user-scoped persistent stores (connections, query history,
 * saved queries) in parallel. Shared between the onboarding and unlock
 * flows since both transition to `ready` and need the same hydration.
 */
function useLoadUserData(): () => Promise<void> {
  const loadConnections = useAppStore((state) => state.loadConnections);
  const loadQueryHistory = useAppStore((state) => state.loadQueryHistory);
  const loadSavedQueries = useAppStore((state) => state.loadSavedQueries);
  return useCallback(async () => {
    await Promise.all([
      loadConnections(),
      loadQueryHistory(),
      loadSavedQueries(),
    ]);
  }, [loadConnections, loadQueryHistory, loadSavedQueries]);
}

const MODE_COPY: Record<
  CredentialStorageMode,
  {
    title: string;
    body: string;
    icon: typeof IconShieldLock;
    recommended?: boolean;
  }
  // oxlint-disable-next-line anti-slop/no-known-value-widening -- The value is handled at a typed library or domain boundary here.
> = {
  "encrypted-sqlite": {
    title: "Encrypted SQLite",
    body: "Stores database passwords in dbunk.sqlite, encrypted with an app password you enter once per session.",
    icon: IconShieldLock,
    recommended: true,
  },
  keychain: {
    title: "OS keychain",
    body: "Uses your operating system credential store. Secure, but macOS may ask for permission after rebuilds.",
    icon: IconKey,
  },
  "plain-sqlite": {
    title: "Unencrypted SQLite",
    body: "Stores database passwords as plaintext in dbunk.sqlite. Convenient, but anyone with file access can read them.",
    icon: IconDatabase,
  },
};

export function CredentialOnboarding() {
  const [mode, setMode] = useState<CredentialStorageMode>("encrypted-sqlite");
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [acknowledged, setAcknowledged] = useState(false);

  const configureCredentialStorage = useAppStore(
    (state) => state.configureCredentialStorage,
  );
  const loadUserData = useLoadUserData();
  const status = useAppStore((state) => state.credentialStorageStatus);

  const needsPassword = mode === "encrypted-sqlite";
  const needsAck = mode === "encrypted-sqlite" || mode === "plain-sqlite";
  const canContinue =
    (!needsPassword || (password.length > 0 && password === confirmPassword)) &&
    (!needsAck || acknowledged) &&
    status.state !== "running";

  const handleContinue = async () => {
    const snapshot = await configureCredentialStorage({
      mode,
      password: needsPassword ? password : undefined,
    });
    if (snapshot?.credentialState === "ready") await loadUserData();
  };

  return (
    <div className="flex h-full w-full items-center justify-center bg-surface-app p-6 text-foreground">
      <div className="grid w-full max-w-5xl gap-6 lg:grid-cols-[0.9fr_1.1fr]">
        <section className="flex flex-col justify-between rounded-lg border border-border-subtle bg-surface-window p-6">
          <div>
            <div className="flex size-10 items-center justify-center rounded-md border border-accent/30 bg-accent/10 text-accent">
              <IconLock className="size-5" />
            </div>
            <h1 className="mt-5 text-2xl font-semibold tracking-tight">
              Set up dbunk
            </h1>
            <p className="mt-2 max-w-sm text-sm leading-6 text-text-muted">
              Choose how dbunk should store saved database passwords before the
              workspace loads connections or starts health checks.
            </p>
          </div>
          <div className="mt-8 grid gap-3 text-xs text-text-muted">
            <div>Local settings live in ~/.config/dbunk/dbunk.sqlite.</div>
            <div>
              Connection metadata stays readable; passwords follow the mode you
              choose.
            </div>
            <div>You can change this later from system Settings.</div>
          </div>
        </section>

        <section className="rounded-lg border border-border-subtle bg-surface-window p-5">
          <div className="grid gap-3">
            {/* oxlint-disable-next-line anti-slop/require-safety-comment-for-type-assertion -- MODE_COPY is keyed by every CredentialStorageMode value. */}
            {(Object.keys(MODE_COPY) as CredentialStorageMode[]).map((id) => {
              const item = MODE_COPY[id];
              const Icon = item.icon;
              const selected = mode === id;
              return (
                <button
                  key={id}
                  type="button"
                  onClick={() => {
                    setMode(id);
                    setAcknowledged(false);
                  }}
                  className={cn(
                    "flex items-start gap-3 rounded-lg border p-4 text-left transition-colors",
                    selected
                      ? "border-accent/50 bg-accent/10"
                      : "border-border-subtle bg-surface-panel hover:border-border-strong",
                  )}
                >
                  <span className="flex size-9 shrink-0 items-center justify-center rounded-md border border-border-subtle bg-surface-window text-text-secondary">
                    <Icon className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2 text-sm font-semibold">
                      {item.title}
                      {item.recommended ? (
                        <span className="rounded-md bg-accent/15 px-1.5 py-0.5 text-[0.625rem] text-accent">
                          Recommended
                        </span>
                      ) : null}
                    </span>
                    <span className="mt-1 block text-xs leading-5 text-text-muted">
                      {item.body}
                    </span>
                  </span>
                </button>
              );
            })}
          </div>

          {needsPassword ? (
            <div className="mt-4 grid gap-3 rounded-lg border border-border-subtle bg-surface-panel p-4">
              <Input
                type="password"
                placeholder="Credential password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
              />
              <Input
                type="password"
                placeholder="Confirm credential password"
                value={confirmPassword}
                onChange={(event) => setConfirmPassword(event.target.value)}
              />
              {confirmPassword && password !== confirmPassword ? (
                <div className="text-xs text-danger">
                  Passwords do not match.
                </div>
              ) : null}
            </div>
          ) : null}

          {needsAck ? (
            <label className="mt-4 flex items-start gap-2 rounded-lg border border-border-subtle bg-surface-panel p-3 text-xs leading-5 text-text-muted">
              <input
                type="checkbox"
                className="mt-1"
                checked={acknowledged}
                onChange={(event) => setAcknowledged(event.target.checked)}
              />
              <span>
                {mode === "encrypted-sqlite"
                  ? "I understand this password cannot be recovered. If I forget it, dbunk must reset credential storage and saved database passwords will be lost."
                  : "I understand database passwords will be stored as plaintext in the local SQLite database."}
              </span>
            </label>
          ) : null}

          {status.state === "error" ? (
            <div className="mt-4 rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
              {status.error}
            </div>
          ) : null}

          <div className="mt-5 flex justify-end">
            <Button
              type="button"
              disabled={!canContinue}
              onClick={handleContinue}
            >
              {status.state === "running" ? "Configuring..." : "Continue"}
            </Button>
          </div>
        </section>
      </div>
    </div>
  );
}

export function CredentialUnlock() {
  const [password, setPassword] = useState("");
  const unlockCredentials = useAppStore((state) => state.unlockCredentials);
  const resetCredentialStorage = useAppStore(
    (state) => state.resetCredentialStorage,
  );
  const loadUserData = useLoadUserData();
  const status = useAppStore((state) => state.credentialStorageStatus);

  const handleUnlock = async () => {
    const snapshot = await unlockCredentials(password);
    if (snapshot?.credentialState === "ready") await loadUserData();
  };

  return (
    <div className="flex h-full w-full items-center justify-center bg-surface-app p-6 text-foreground">
      <div className="w-full max-w-md rounded-lg border border-border-subtle bg-surface-window p-6">
        <div className="flex size-10 items-center justify-center rounded-md border border-accent/30 bg-accent/10 text-accent">
          <IconShieldLock className="size-5" />
        </div>
        <h1 className="mt-5 text-xl font-semibold">Unlock credentials</h1>
        <p className="mt-2 text-sm leading-6 text-text-muted">
          Enter your dbunk credential password to load saved connections and
          start health checks.
        </p>
        <Input
          className="mt-5"
          type="password"

          placeholder="Credential password"
          value={password}
          onChange={(event) => setPassword(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && password) {
              void handleUnlock();
            }
          }}
        />
        {status.state === "error" ? (
          <div className="mt-4 rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-xs text-danger">
            {status.error}
          </div>
        ) : null}
        <div className="mt-5 flex items-center justify-between gap-3">
          <Button
            type="button"
            variant="ghost"
            className="text-text-muted"
            onClick={() => {
              if (
                window.confirm(
                  "Reset credential storage? All query sessions will close, active transactions will roll back, and saved database passwords will be deleted permanently. Connection profiles and other app data remain.",
                )
              ) {
                void resetCredentialStorage();
              }
            }}
          >
            Forgot password?
          </Button>
          <Button
            type="button"
            disabled={!password || status.state === "running"}
            onClick={handleUnlock}
          >
            {status.state === "running" ? "Unlocking..." : "Unlock"}
          </Button>
        </div>
      </div>
    </div>
  );
}

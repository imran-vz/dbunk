import {
  IconAdjustments,
  IconBox,
  IconDatabase,
  IconInfoCircle,
  IconKey,
  IconMoon,
  IconServer,
  IconShieldLock,
  IconSun,
} from "@tabler/icons-react";
import { useState } from "react";

import { BastionServersTab } from "@/components/bastion-servers-tab";
import { ConnectionsView } from "@/components/connections-view";
import { ManagedServersTab } from "@/components/managed-servers-tab";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { CredentialStorageMode, SettingsTab } from "@/lib/store";
import { useAppStore } from "@/lib/store";
import {
  isPresetIntrinsicallyDark,
  isThemeMode,
  isThemePreset,
  type ThemeMode,
  type ThemePreset,
} from "@/lib/theme";
import { cn } from "@/lib/utils";

const TABS: Array<{
  id: SettingsTab;
  label: string;
  icon: typeof IconAdjustments;
}> = [
  { id: "general", label: "General", icon: IconAdjustments },
  { id: "connections", label: "Connections", icon: IconDatabase },
  { id: "bastions", label: "Bastion Servers", icon: IconServer },
  { id: "local-databases", label: "Local Databases", icon: IconBox },
  { id: "security", label: "Security", icon: IconShieldLock },
  { id: "about", label: "About", icon: IconInfoCircle },
];

export function SettingsView() {
  const activeTab = useAppStore((state) => state.settingsTab);
  const setSettingsTab = useAppStore((state) => state.setSettingsTab);

  return (
    <div className="flex h-full min-h-0 flex-1 flex-row bg-surface-app">
      <nav
        aria-label="Settings sections"
        className="flex w-48 shrink-0 flex-col gap-0.5 border-r border-border-subtle bg-surface-sidebar px-2 py-4"
      >
        <div className="px-2 pb-2 text-[0.65rem] uppercase tracking-wider text-text-muted">
          Settings
        </div>
        {TABS.map((tab) => {
          const Icon = tab.icon;
          const isActive = activeTab === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              onClick={() => setSettingsTab(tab.id)}
              className={cn(
                "flex h-8 items-center gap-2 rounded-md px-2 text-xs text-text-secondary transition-colors",
                isActive
                  ? "bg-accent-green/10 text-foreground"
                  : "hover:bg-surface-panel hover:text-foreground",
              )}
            >
              <Icon className="size-3.5" />
              {tab.label}
            </button>
          );
        })}
      </nav>
      <div className="flex min-h-0 flex-1 flex-col">
        {activeTab === "general" ? <GeneralTab /> : null}
        {activeTab === "connections" ? <ConnectionsView /> : null}
        {activeTab === "bastions" ? <BastionServersTab /> : null}
        {activeTab === "local-databases" ? <ManagedServersTab /> : null}
        {activeTab === "security" ? <SecurityTab /> : null}
        {activeTab === "about" ? <AboutTab /> : null}
      </div>
    </div>
  );
}

const THEME_MODES: Array<{
  id: ThemeMode;
  label: string;
  description: string;
  icon: typeof IconSun;
}> = [
  {
    id: "system",
    label: "System",
    description: "Follow the OS appearance setting.",
    icon: IconSun,
  },
  {
    id: "light",
    label: "Light",
    description: "Always use the light palette.",
    icon: IconSun,
  },
  {
    id: "dark",
    label: "Dark",
    description: "Always use the dark palette.",
    icon: IconMoon,
  },
];

const THEME_PRESETS: Array<{
  id: ThemePreset;
  label: string;
  description: string;
}> = [
  {
    id: "default",
    label: "Default",
    description: "Stock dbunk palette.",
  },
  {
    id: "dracula",
    label: "Dracula",
    description: "Classic dracula palette. Always dark.",
  },
  {
    id: "github",
    label: "GitHub",
    description: "Primer-inspired light + dimmed dark.",
  },
  {
    id: "gruvbox",
    label: "Gruvbox",
    description: "Warm cream / hard-contrast dark.",
  },
];

function GeneralTab() {
  const theme: ThemeMode = useAppStore(
    (state) => state.appSettings?.theme ?? "system",
  );
  const preset: ThemePreset = useAppStore(
    (state) => state.appSettings?.themePreset ?? "default",
  );
  const setTheme = useAppStore((state) => state.setTheme);
  const setThemePreset = useAppStore((state) => state.setThemePreset);

  const modeLocked = isPresetIntrinsicallyDark(preset);

  return (
    <>
      <SectionHeader
        title="General"
        subtitle="Appearance preferences for the dbunk workspace."
      />
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <div className="mx-auto flex max-w-3xl flex-col gap-6">
          <section className="rounded-lg border border-border-subtle bg-surface-window p-5">
            <h2 className="text-sm font-semibold text-foreground">Mode</h2>
            <p className="mt-1 text-xs text-text-muted">
              {modeLocked
                ? "The current preset is intrinsically dark — mode is locked until you switch presets."
                : "Choose between system, light, and dark."}
            </p>
            <div className="mt-4 grid gap-2 sm:grid-cols-3">
              {THEME_MODES.map((option) => {
                const Icon = option.icon;
                const active = theme === option.id;
                return (
                  <button
                    key={option.id}
                    type="button"
                    disabled={modeLocked}
                    onClick={() => {
                      if (!isThemeMode(option.id)) return;
                      void setTheme(option.id);
                    }}
                    className={cn(
                      "flex flex-col items-start gap-1.5 rounded-lg border p-3 text-left transition-colors",
                      active
                        ? "border-accent-green/50 bg-accent-green/10"
                        : "border-border-subtle bg-surface-panel hover:border-border-strong",
                      modeLocked && !active ? "opacity-50" : "",
                    )}
                  >
                    <div className="flex items-center gap-2 text-sm font-medium">
                      <Icon className="size-3.5" />
                      {option.label}
                    </div>
                    <div className="text-xs text-text-muted">
                      {option.description}
                    </div>
                  </button>
                );
              })}
            </div>
          </section>

          <section className="rounded-lg border border-border-subtle bg-surface-window p-5">
            <h2 className="text-sm font-semibold text-foreground">Preset</h2>
            <p className="mt-1 text-xs text-text-muted">
              Swap the entire colour palette. Mode (above) still controls light
              / dark for presets that support both.
            </p>
            <div className="mt-4 grid gap-2 sm:grid-cols-2">
              {THEME_PRESETS.map((option) => {
                const active = preset === option.id;
                return (
                  <button
                    key={option.id}
                    type="button"
                    onClick={() => {
                      if (!isThemePreset(option.id)) return;
                      void setThemePreset(option.id);
                    }}
                    className={cn(
                      "flex flex-col items-start gap-1.5 rounded-lg border p-3 text-left transition-colors",
                      active
                        ? "border-accent-green/50 bg-accent-green/10"
                        : "border-border-subtle bg-surface-panel hover:border-border-strong",
                    )}
                  >
                    <div className="text-sm font-medium">{option.label}</div>
                    <div className="text-xs text-text-muted">
                      {option.description}
                    </div>
                  </button>
                );
              })}
            </div>
          </section>
        </div>
      </div>
    </>
  );
}

const STORAGE_OPTIONS: Array<{
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

function storageLabelFor(mode: CredentialStorageMode): string {
  return STORAGE_OPTIONS.find((option) => option.id === mode)?.label ?? mode;
}

function SecurityTab() {
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
    const selectedOption = STORAGE_OPTIONS.find(
      (option) => option.id === selected,
    );
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
    <>
      <SectionHeader
        title="Security"
        subtitle="Where dbunk stores the passwords for your saved database connections."
      />
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <section className="mx-auto max-w-3xl rounded-lg border border-border-subtle bg-surface-window p-5">
          <div className="flex flex-wrap items-start justify-between gap-3 border-b border-border-subtle pb-4">
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                Credential storage
              </h2>
              <p className="mt-1 text-xs leading-5 text-text-muted">
                Current mode: {storageLabelFor(current)}. Settings are stored in{" "}
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
            {STORAGE_OPTIONS.map((option) => {
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
    </>
  );
}

function AboutTab() {
  const configDir = useAppStore(
    (state) => state.appSettings?.configDir ?? "~/.config/dbunk",
  );
  return (
    <>
      <SectionHeader
        title="About"
        subtitle="Build details and project links."
      />
      <div className="min-h-0 flex-1 overflow-auto p-6">
        <section className="mx-auto max-w-3xl rounded-lg border border-border-subtle bg-surface-window p-5 text-xs">
          <dl className="grid grid-cols-[8rem_1fr] gap-y-2 text-text-secondary">
            <dt className="font-medium text-text-muted">Application</dt>
            <dd>dbunk</dd>
            <dt className="font-medium text-text-muted">Repository</dt>
            <dd>
              <a
                href="https://github.com/imran-vz/dbunk"
                target="_blank"
                rel="noreferrer"
                className="text-info hover:underline"
              >
                github.com/imran-vz/dbunk
              </a>
            </dd>
            <dt className="font-medium text-text-muted">Config directory</dt>
            <dd>
              <code className="rounded bg-surface-panel-elevated px-1.5 py-0.5">
                {configDir}
              </code>
            </dd>
          </dl>
          <p className="mt-4 text-text-muted">
            Cloud sign-in is coming in a future release.
          </p>
        </section>
      </div>
    </>
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

import { IconRefresh, IconSearch } from "@tabler/icons-react";
import { type ReactNode, useEffect, useMemo, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardAction,
  CardContent,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import type {
  Connection,
  PgExtension,
  PgSetting,
  ServerDetails,
  ServerDetailsStatus,
} from "@/lib/store";
import { cn } from "@/lib/utils";

import { KeyValue } from "./key-value";

export function DetailsTab({
  activeConnection,
  details,
  status,
  onLoad,
}: {
  activeConnection: Connection;
  details: ServerDetails | undefined;
  status: ServerDetailsStatus | undefined;
  onLoad: (connectionId: string) => Promise<void>;
}) {
  useEffect(() => {
    const state = status?.state;
    if (state === "loading" || state === "success") {
      return;
    }
    void onLoad(activeConnection.id);
  }, [activeConnection.id, onLoad, status?.state]);

  const refreshing = status?.state === "loading";

  return (
    <div className="flex flex-col gap-4">
      <Card>
        <CardHeader>
          <CardTitle>Server</CardTitle>
          <CardAction>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void onLoad(activeConnection.id)}
              disabled={refreshing}
            >
              <IconRefresh
                className={cn("size-3.5", refreshing && "animate-spin")}
              />
              {refreshing ? "Refreshing…" : "Refresh"}
            </Button>
          </CardAction>
        </CardHeader>
        <CardContent className="grid grid-cols-2 gap-x-6 gap-y-3 text-xs sm:grid-cols-3">
          <KeyValue
            label="Server version"
            value={
              details?.serverVersion ? trimVersion(details.serverVersion) : "—"
            }
          />
          <KeyValue label="Encoding" value={details?.encoding || "—"} />
          <KeyValue label="Locale" value={details?.locale || "—"} />
          <KeyValue label="Timezone" value={details?.timezone || "—"} />
        </CardContent>
      </Card>

      {status?.state === "error" ? (
        <div className="rounded-md border border-dashed border-danger/40 bg-danger/5 px-3 py-6 text-center text-xs text-danger">
          Failed to load server details — {status.error}
        </div>
      ) : null}

      <SettingsCard settings={details?.settings ?? []} loading={refreshing} />

      <ExtensionsCard
        extensions={details?.extensions ?? []}
        loading={refreshing}
      />
    </div>
  );
}

/**
 * Trim `version()`'s long fingerprint down to the part users care
 * about. Postgres returns e.g.
 * "PostgreSQL 16.2 (Debian 16.2-1.pgdg120+2) on x86_64-pc-linux-gnu, …";
 * the parens-and-on suffix is mostly platform noise.
 */
function trimVersion(raw: string): string {
  const onIdx = raw.indexOf(" on ");
  return onIdx > 0 ? raw.slice(0, onIdx) : raw;
}

type SettingsCardProps = {
  settings: PgSetting[];
  loading: boolean;
};

function SettingsCard({ settings, loading }: SettingsCardProps) {
  const [searchText, setSearchText] = useState("");
  const [onlyOverridden, setOnlyOverridden] = useState(false);

  const filtered = useMemo(() => {
    const needle = searchText.trim().toLowerCase();
    return settings.filter((setting) => {
      if (onlyOverridden && setting.source === "default") {
        return false;
      }
      if (!needle) {
        return true;
      }
      return (
        setting.name.toLowerCase().includes(needle) ||
        setting.category.toLowerCase().includes(needle) ||
        (setting.shortDesc?.toLowerCase().includes(needle) ?? false)
      );
    });
  }, [settings, searchText, onlyOverridden]);

  const grouped = useMemo(() => groupByCategory(filtered), [filtered]);

  return (
    <Card>
      <CardHeader>
        <CardTitle>Settings</CardTitle>
        <CardAction>
          <button
            type="button"
            onClick={() => setOnlyOverridden((prev) => !prev)}
            className={cn(
              "rounded-md border px-2.5 py-1 text-[0.6875rem] font-medium transition-colors",
              onlyOverridden
                ? "border-accent-green/40 bg-accent-green/10 text-accent-green-hover"
                : "border-border-subtle text-text-muted hover:bg-surface-panel hover:text-foreground",
            )}
          >
            Modified only
          </button>
        </CardAction>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 text-xs">
        <div className="relative">
          <IconSearch className="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-text-muted" />
          <Input
            value={searchText}
            onChange={(event) => setSearchText(event.currentTarget.value)}
            placeholder="Search by name, category, or description…"
            className="pl-7"
          />
        </div>

        <div className="text-[0.625rem] text-text-muted">
          Showing {filtered.length} of {settings.length} parameters
          {onlyOverridden ? " (modified only)" : ""}.
        </div>

        {filtered.length === 0 ? (
          <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
            {loading
              ? "Loading settings…"
              : settings.length === 0
                ? "No settings available."
                : "No settings match the current filters."}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {grouped.map(({ category, rows }) => (
              <CategoryGroup key={category} category={category} rows={rows} />
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function groupByCategory(
  settings: PgSetting[],
): Array<{ category: string; rows: PgSetting[] }> {
  const map = new Map<string, PgSetting[]>();
  for (const setting of settings) {
    const list = map.get(setting.category);
    if (list) {
      list.push(setting);
    } else {
      map.set(setting.category, [setting]);
    }
  }
  return Array.from(map.entries()).map(([category, rows]) => ({
    category,
    rows,
  }));
}

function CategoryGroup({
  category,
  rows,
}: {
  category: string;
  rows: PgSetting[];
}) {
  return (
    <div className="overflow-hidden rounded-md border border-border-subtle">
      <div className="bg-surface-panel px-3 py-1.5 text-[0.625rem] font-medium uppercase tracking-[0.08em] text-text-muted">
        {category}
      </div>
      <table className="w-full border-collapse text-left text-[0.75rem]">
        <tbody>
          {rows.map((row) => (
            <SettingRow key={row.name} setting={row} />
          ))}
        </tbody>
      </table>
    </div>
  );
}

function SettingRow({ setting }: { setting: PgSetting }) {
  const isModified = setting.source !== "default";
  const displayValue = setting.unit
    ? `${setting.setting} ${setting.unit}`
    : setting.setting;
  return (
    <tr
      className="border-t border-border-subtle first:border-t-0"
      title={setting.shortDesc ?? undefined}
    >
      <td className="w-1/3 px-3 py-1.5 font-mono text-foreground">
        <div className="flex items-center gap-1.5">
          {setting.name}
          {isModified ? (
            <span
              className="rounded-full bg-warning/10 px-1.5 py-0.5 text-[0.5625rem] font-medium uppercase tracking-[0.08em] text-warning"
              title={`Source: ${setting.source}`}
            >
              modified
            </span>
          ) : null}
        </div>
      </td>
      <td className="px-3 py-1.5 font-mono text-foreground">{displayValue}</td>
      <td className="px-3 py-1.5 text-right tabular-nums text-text-muted">
        {setting.source}
      </td>
    </tr>
  );
}

type ExtensionsCardProps = {
  extensions: PgExtension[];
  loading: boolean;
};

function ExtensionsCard({ extensions, loading }: ExtensionsCardProps) {
  return (
    <Card>
      <CardHeader>
        <CardTitle>Installed extensions</CardTitle>
      </CardHeader>
      <CardContent className="text-xs">
        {extensions.length === 0 ? (
          <div className="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-text-muted">
            {loading
              ? "Loading extensions…"
              : "No extensions installed on this database."}
          </div>
        ) : (
          <div className="overflow-hidden rounded-md border border-border-subtle">
            <table className="w-full border-collapse text-left text-[0.75rem]">
              <thead className="bg-surface-panel text-[0.625rem] uppercase tracking-[0.08em] text-text-muted">
                <tr>
                  <Th>Name</Th>
                  <Th>Version</Th>
                  <Th>Schema</Th>
                  <Th>Description</Th>
                </tr>
              </thead>
              <tbody>
                {extensions.map((ext) => (
                  <tr key={ext.name} className="border-t border-border-subtle">
                    <td className="px-3 py-1.5 font-mono text-foreground">
                      {ext.name}
                    </td>
                    <td className="px-3 py-1.5 tabular-nums text-text-muted">
                      {ext.version}
                    </td>
                    <td className="px-3 py-1.5 text-text-muted">
                      {ext.schema}
                    </td>
                    <td className="px-3 py-1.5 text-text-muted">
                      {ext.description ?? "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function Th({ children }: { children: ReactNode }) {
  return <th className="px-3 py-1.5 font-medium">{children}</th>;
}

import { IconDatabaseOff } from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { StatusDot } from "@/components/ui/status-dot";
import type { OverviewTabId } from "@/lib/store";
import { cn } from "@/lib/utils";

type OverviewTab = { id: OverviewTabId; label: string };

export const OVERVIEW_TABS: readonly OverviewTab[] = [
  { id: "overview", label: "Overview" },
  { id: "tables", label: "Tables" },
  { id: "schemas", label: "Schemas" },
  { id: "schema-map", label: "Schema Map" },
  { id: "query-history", label: "Query History" },
  { id: "admin", label: "Admin" },
  { id: "compare", label: "Compare" },
  { id: "details", label: "Details" },
  { id: "settings", label: "Settings" },
] as const;

export function OverviewHeader({
  name,
  activeTab,
  onTabChange,
  onDisconnect,
}: {
  name: string;
  activeTab: OverviewTabId;
  onTabChange: (tab: OverviewTabId) => void;
  onDisconnect: () => void;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-3 border-b border-border-subtle pb-3">
      <div className="min-w-0">
        <div className="flex items-center gap-2.5">
          <h1 className="text-xl font-semibold tracking-tight text-foreground">
            {name}
          </h1>
          <span className="flex items-center gap-1.5 text-xs font-medium text-accent-hover">
            <StatusDot tone="healthy" className="size-2" />
            Connected
          </span>
        </div>
        <nav
          aria-label="Connection sections"
          className="mt-3 flex flex-wrap items-center gap-1 text-xs"
        >
          {OVERVIEW_TABS.map((tab) => {
            const isActive = tab.id === activeTab;
            return (
              <button
                key={tab.id}
                type="button"
                aria-current={isActive ? "page" : undefined}
                onClick={() => onTabChange(tab.id)}
                className={cn(
                  "rounded-md px-2.5 py-1 transition-colors",
                  isActive
                    ? "bg-accent/10 text-accent-hover"
                    : "text-text-muted hover:bg-surface-panel hover:text-foreground",
                )}
              >
                {tab.label}
              </button>
            );
          })}
        </nav>
      </div>
      <Button type="button" size="sm" variant="outline" onClick={onDisconnect}>
        <IconDatabaseOff className="size-3.5" />
        Disconnect
      </Button>
    </header>
  );
}

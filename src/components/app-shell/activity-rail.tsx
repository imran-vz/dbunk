import {
  IconBroadcast,
  IconChartBar,
  IconHistory,
  IconKey,
  IconServer,
  IconSettings,
  IconShare3,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";

import {
  MACOS_TRAFFIC_LIGHT_GUTTER_PX,
  needsMacTitlebarGutter,
} from "@/components/app-shell/macos-titlebar";
import { cn } from "@/lib/utils";

export type WorkbenchRailId =
  | "tables"
  | "queries"
  | "schema-map"
  | "history"
  | "admin";

export type KeyValueRailId = "keys" | "cli" | "pubsub" | "server";

export interface RailItem<T extends string> {
  id: T;
  icon: typeof IconTable;
  label: string;
}

export const RELATIONAL_RAIL_ITEMS: ReadonlyArray<RailItem<WorkbenchRailId>> = [
  { id: "tables", icon: IconTable, label: "Tables" },
  { id: "queries", icon: IconTerminal2, label: "Queries" },
  { id: "schema-map", icon: IconShare3, label: "Schema map" },
  { id: "history", icon: IconHistory, label: "History" },
  { id: "admin", icon: IconChartBar, label: "Admin" },
];

export const KEYVALUE_RAIL_ITEMS: ReadonlyArray<RailItem<KeyValueRailId>> = [
  { id: "keys", icon: IconKey, label: "Keys" },
  { id: "cli", icon: IconTerminal2, label: "CLI" },
  { id: "pubsub", icon: IconBroadcast, label: "Pub/Sub" },
  { id: "server", icon: IconServer, label: "Server" },
];

interface ActivityRailProps<T extends string> {
  items: ReadonlyArray<RailItem<T>>;
  active: T;
  onChange: (id: T) => void;
  onOpenSettings: () => void;
  isWindowFullscreen?: boolean;
  onPointerDown?: React.PointerEventHandler<HTMLElement>;
  onDoubleClick?: React.MouseEventHandler<HTMLElement>;
  className?: string;
}

export function ActivityRail<T extends string>({
  items,
  active,
  onChange,
  onOpenSettings,
  isWindowFullscreen = false,
  onPointerDown,
  onDoubleClick,
  className,
}: ActivityRailProps<T>) {
  const macGutter = needsMacTitlebarGutter(isWindowFullscreen);

  return (
    <nav
      aria-label="Workbench sections"
      className={cn(
        "flex w-12 shrink-0 flex-col items-center gap-1 border-r border-border-subtle bg-surface-sidebar py-2",
        className,
      )}
    >
      {macGutter ? (
        <div
          aria-hidden="true"
          data-window-drag-region
          className="w-full shrink-0"
          style={{ height: MACOS_TRAFFIC_LIGHT_GUTTER_PX }}
          onPointerDown={onPointerDown}
          onDoubleClick={onDoubleClick}
        />
      ) : null}
      <div
        aria-hidden="true"
        className="mb-2 flex size-7 items-center justify-center rounded-md bg-accent text-sm font-bold text-accent-foreground"
      >
        d
      </div>
      {items.map((item) => {
        const Icon = item.icon;
        const on = active === item.id;
        return (
          <button
            key={item.id}
            type="button"
            title={item.label}
            aria-label={item.label}
            aria-current={on ? "page" : undefined}
            onClick={() => onChange(item.id)}
            className={cn(
              "relative flex size-9 items-center justify-center rounded-md transition-colors",
              on
                ? "bg-accent-subdued text-accent"
                : "text-text-disabled hover:text-text-muted",
            )}
          >
            {on ? (
              <span
                aria-hidden="true"
                className="absolute left-0 h-5 w-0.5 rounded-full bg-accent"
              />
            ) : null}
            <Icon className="size-5" />
          </button>
        );
      })}
      <div className="mt-auto">
        <button
          type="button"
          title="Settings"
          aria-label="Settings"
          onClick={onOpenSettings}
          className="flex size-9 items-center justify-center rounded-md text-text-disabled transition-colors hover:text-text-muted"
        >
          <IconSettings className="size-5" />
        </button>
      </div>
    </nav>
  );
}

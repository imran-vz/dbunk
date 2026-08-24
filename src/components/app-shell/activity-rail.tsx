import {
  IconBroadcast,
  IconChartBar,
  IconHistory,
  IconKey,
  IconLayoutDashboard,
  IconPlug,
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
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export type WorkbenchRailId =
  | "connections"
  | "tables"
  | "queries"
  | "history"
  | "schema-map"
  | "admin"
  | "overview";

export type KeyValueRailId = "keys" | "cli" | "pubsub" | "server";

export interface RailItem<T extends string> {
  id: T;
  icon: typeof IconTable;
  label: string;
}

/** Rail items per decision D4; Settings is pinned at the bottom. */
export const RELATIONAL_RAIL_ITEMS: ReadonlyArray<RailItem<WorkbenchRailId>> = [
  { id: "connections", icon: IconPlug, label: "Connections" },
  { id: "tables", icon: IconTable, label: "Tables" },
  { id: "queries", icon: IconTerminal2, label: "Queries" },
  { id: "history", icon: IconHistory, label: "History" },
  { id: "schema-map", icon: IconShare3, label: "Schema map" },
  { id: "admin", icon: IconChartBar, label: "Admin" },
  { id: "overview", icon: IconLayoutDashboard, label: "Overview" },
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

function RailButton({
  icon: Icon,
  label,
  active = false,
  onClick,
}: {
  icon: typeof IconTable;
  label: string;
  active?: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            aria-label={label}
            aria-current={active ? "page" : undefined}
            onClick={onClick}
            className={cn(
              // 28px hit target on the 40px rail (§3.1); icons 18px.
              "relative size-auto h-7 w-7 rounded-sm p-0 [&_svg:not([class*='size-'])]:size-4.5",
              active
                ? "bg-accent-subdued text-accent hover:bg-accent-subdued hover:text-accent"
                : "text-text-disabled hover:bg-transparent hover:text-text-muted",
            )}
          />
        }
      >
        {active ? (
          <span
            aria-hidden="true"
            className="absolute top-1/2 -left-1.5 h-4 w-0.5 -translate-y-1/2 bg-accent"
          />
        ) : null}
        <Icon />
      </TooltipTrigger>
      <TooltipContent side="right">{label}</TooltipContent>
    </Tooltip>
  );
}

/**
 * The permanent 40px activity rail (§3.1). Never collapses; doubles
 * as the navigator's restore affordance.
 */
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
        "flex w-10 shrink-0 flex-col items-center gap-1 border-r border-border-subtle bg-surface-sidebar py-2",
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
        className="mb-2 flex size-6 items-center justify-center rounded-sm bg-accent text-xs font-bold text-accent-foreground"
      >
        d
      </div>
      {items.map((item) => (
        <RailButton
          key={item.id}
          icon={item.icon}
          label={item.label}
          active={active === item.id}
          onClick={() => onChange(item.id)}
        />
      ))}
      <div className="mt-auto">
        <RailButton
          icon={IconSettings}
          label="Settings"
          onClick={onOpenSettings}
        />
      </div>
    </nav>
  );
}

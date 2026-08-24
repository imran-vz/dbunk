/**
 * Global console dock (DESIGN-SYSTEM §5.6) — the full-width bottom
 * surface for app-wide streams: connection lifecycle, server notices,
 * task/export progress, and the cross-tab query log.
 *
 * Hidden by default and it never auto-opens: new events while hidden
 * increment the status-bar badge. Toggled by `` Ctrl+` `` or the badge.
 * Height is resizable on the shared sash spec and persists globally.
 */

import { IconArrowDown, IconTrash, IconX } from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { usePanelState } from "@/components/ui/panel";
import { Sash } from "@/components/ui/resizer-handle";
import { Segmented } from "@/components/ui/segmented";
import { EmptyState } from "@/components/ui/state-panel";
import { StatusDot, type StatusTone } from "@/components/ui/status-dot";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { shortcutKeys, useShortcutHandler } from "@/lib/shortcuts";
import {
  type ConsoleEvent,
  type ConsoleSeverity,
  useAppStore,
} from "@/lib/store";
import { cn } from "@/lib/utils";

/** §3.3: dock max = 60% of the window height. */
const DOCK_MAX = () => Math.round(window.innerHeight * 0.6);

type SeverityFilter = "all" | ConsoleSeverity;

const severityTone = {
  info: "neutral",
  warning: "warning",
  error: "danger",
} satisfies Record<ConsoleSeverity, StatusTone>;

const timeOf = (iso: string): string => {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? "--:--:--"
    : date.toTimeString().slice(0, 8);
};

export function GlobalConsoleDock() {
  const { consoleEvents, dockOpen, setDockOpen, toggleDock, clearConsole } =
    useAppStore();
  const [filter, setFilter] = useState<SeverityFilter>("all");
  const [follow, setFollow] = useState(true);
  const scrollRef = useRef<HTMLDivElement>(null);

  const height = usePanelState({
    storageKey: "dbunk.panel.dock",
    defaultSize: 200,
    min: 100,
    max: DOCK_MAX,
    snapThreshold: 60,
  });

  useShortcutHandler("toggle-console", toggleDock);

  // `` Ctrl+` `` toggles the dock (§6.1).
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey && !event.metaKey && event.key === "`") {
        event.preventDefault();
        toggleDock();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [toggleDock]);

  const visibleEvents =
    filter === "all"
      ? consoleEvents
      : consoleEvents.filter((event) => event.severity === filter);

  // Auto-scroll to the tail while following.
  // oxlint-disable-next-line exhaustive-deps -- scrolls on every new event by design.
  useEffect(() => {
    if (!dockOpen || !follow) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [dockOpen, follow, visibleEvents.length, filter]);

  if (!dockOpen) return null;

  return (
    <div
      data-slot="global-console-dock"
      data-testid="global-console-dock"
      className="relative flex shrink-0 flex-col overflow-hidden border-t border-border-subtle bg-surface-panel"
      style={{ height: height.size }}
    >
      <Sash
        orientation="horizontal"
        side="top"
        value={height.size}
        min={height.min}
        max={height.max}
        snapThreshold={height.snapThreshold}
        onResize={height.setSize}
        onCollapse={() => setDockOpen(false)}
        ariaLabel="Resize console"
      />
      <div className="flex h-8 shrink-0 items-center gap-2 border-b border-border-subtle px-2">
        <span className="text-2xs font-semibold uppercase tracking-[0.12em] text-text-muted">
          Console
        </span>
        <Segmented<SeverityFilter>
          value={filter}
          onChange={setFilter}
          options={[
            { id: "all", label: "All" },
            { id: "info", label: "Info" },
            { id: "warning", label: "Warnings" },
            { id: "error", label: "Errors" },
          ]}
        />
        <div className="ml-auto flex items-center gap-1">
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={
                    follow ? "Stop following output" : "Follow output"
                  }
                  aria-pressed={follow}
                  onClick={() => setFollow((current) => !current)}
                  className={cn(follow && "bg-accent-subdued text-accent")}
                />
              }
            >
              <IconArrowDown />
            </TooltipTrigger>
            <TooltipContent>
              {follow ? "Stop following output" : "Follow output"}
            </TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label="Clear console"
                  onClick={clearConsole}
                />
              }
            >
              <IconTrash />
            </TooltipTrigger>
            <TooltipContent>Clear console</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger
              render={
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label="Hide console"
                  onClick={() => setDockOpen(false)}
                />
              }
            >
              <IconX />
            </TooltipTrigger>
            <TooltipContent kbd={shortcutKeys("toggle-console")}>
              Hide console
            </TooltipContent>
          </Tooltip>
        </div>
      </div>
      <div
        ref={scrollRef}
        className="min-h-0 flex-1 overflow-auto"
        onScroll={(event) => {
          const el = event.currentTarget;
          const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 4;
          setFollow(atBottom);
        }}
      >
        {visibleEvents.length === 0 ? (
          <EmptyState
            title={
              filter === "all"
                ? "No console output yet"
                : "No events at this severity"
            }
          />
        ) : (
          <ul className="px-2 py-1">
            {visibleEvents.map((event) => (
              <ConsoleRow key={event.id} event={event} />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

function ConsoleRow({ event }: { event: ConsoleEvent }) {
  return (
    <li className="flex items-start gap-2 border-b border-border-subtle/50 py-1 text-xs last:border-b-0">
      <span className="shrink-0 pt-px font-mono text-2xs tabular-nums text-text-muted">
        {timeOf(event.at)}
      </span>
      <StatusDot
        tone={severityTone[event.severity]}
        className="mt-1.5 size-1.5 shrink-0"
      />
      <span className="shrink-0 pt-px text-2xs uppercase tracking-wide text-text-muted">
        {event.source}
      </span>
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block truncate",
            event.severity === "error" && "text-danger",
            event.severity === "warning" && "text-warning",
            event.severity === "info" && "text-text-secondary",
          )}
        >
          {event.message}
        </span>
        {event.detail ? (
          <span className="block truncate font-mono text-2xs text-text-muted">
            {event.detail}
          </span>
        ) : null}
      </span>
    </li>
  );
}

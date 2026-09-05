import {
  IconChevronDown,
  IconArchive,
  IconColumns3,
  IconEye,
  IconKey,
  IconPlus,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Segmented } from "@/components/ui/segmented";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { confirmCloseQuerySession } from "@/lib/query-session-close";
import {
  type ConnectionEnvironment,
  useAppStore,
  type WorkspaceTab,
} from "@/lib/store";
import { cn } from "@/lib/utils";

const SCROLL_HINT_THRESHOLD_PX = 2;

export type TableSection =
  | "data"
  | "schema"
  | "indexes"
  | "relations"
  | "schema-map"
  | "transfer"
  | "specialized";

interface ObjectTabRowProps {
  sectionControl?: React.ReactNode;
  className?: string;
}

/** Env color for the strip's bottom underline (§4.4). */
const ENV_UNDERLINE = {
  production: "bg-danger",
  staging: "bg-warning",
  test: "bg-info",
} satisfies Partial<Record<ConnectionEnvironment, string>>;

function orderTabs(tabs: WorkspaceTab[]): WorkspaceTab[] {
  // Pinned tabs are leftmost; otherwise stable order.
  const pinned = tabs.filter((tab) => tab.pinned);
  const rest = tabs.filter((tab) => !tab.pinned);
  return [...pinned, ...rest];
}

function WorkspaceTabIcon({
  tab,
  className,
}: {
  tab: WorkspaceTab;
  className?: string;
}) {
  if (tab.kind === "pg-tools") return <IconArchive className={className} />;
  if (tab.kind === "query") return <IconTerminal2 className={className} />;
  if (tab.kind === "object") return <IconEye className={className} />;
  return <IconTable className={className} />;
}

/**
 * Object tab strip (DESIGN-SYSTEM §4.4): `--h-tab` height, natural
 * tab widths with horizontal scroll + trailing chevron on overflow,
 * middle-click close, drag reorder, dirty dot that yields to the
 * close × on hover, per-tab context menu, roving tabindex.
 */
export function ObjectTabRow({ sectionControl, className }: ObjectTabRowProps) {
  const tabsScrollerRef = useRef<HTMLDivElement>(null);
  const [overflowing, setOverflowing] = useState(false);
  const dragTabId = useRef<string | null>(null);
  const connections = useAppStore((state) => state.connections);
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const setActiveTabId = useAppStore((state) => state.setActiveTabId);
  const setWorkspaceTabs = useAppStore((state) => state.setWorkspaceTabs);
  const closeTab = useAppStore((state) => state.closeTab);
  const createNewQueryTab = useAppStore((state) => state.createNewQueryTab);

  const tabs = useMemo(() => orderTabs(workspaceTabs), [workspaceTabs]);
  const hasWorkspaceTabs = tabs.length > 0;

  const environmentById = useMemo(() => {
    const map = new Map<string, ConnectionEnvironment | undefined>();
    for (const connection of connections) {
      map.set(connection.id, connection.environment);
    }
    return map;
  }, [connections]);

  const recomputeOverflow = useCallback(() => {
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    setOverflowing(
      scroller.scrollWidth - scroller.clientWidth > SCROLL_HINT_THRESHOLD_PX,
    );
  }, []);

  useEffect(() => {
    if (!hasWorkspaceTabs) return;
    recomputeOverflow();
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    const resizeObserver =
      // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(recomputeOverflow);
    resizeObserver?.observe(scroller);
    window.addEventListener("resize", recomputeOverflow);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", recomputeOverflow);
    };
  }, [hasWorkspaceTabs, recomputeOverflow, tabs.length]);

  // Keep the active tab scrolled into view.
  useEffect(() => {
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    const active = scroller.querySelector<HTMLElement>(
      '[data-slot="workspace-tab"][aria-selected="true"]',
    );
    active?.scrollIntoView({ inline: "nearest", block: "nearest" });
  }, [activeTabId]);

  const requestClose = useCallback(
    async (tab: WorkspaceTab) => {
      const status =
        useAppStore.getState().querySessions[tab.id]?.transaction.status;
      if (!(await confirmCloseQuerySession(status))) return;
      void closeTab(tab.id);
    },
    [closeTab],
  );

  const closeMany = useCallback(
    async (targets: WorkspaceTab[]) => {
      for (const tab of targets) {
        if (tab.pinned) continue;
        await requestClose(tab);
      }
    },
    [requestClose],
  );

  const togglePinned = useCallback(
    (tabId: string) => {
      setWorkspaceTabs((prev) =>
        prev.map((tab) =>
          tab.id === tabId ? { ...tab, pinned: !tab.pinned } : tab,
        ),
      );
    },
    [setWorkspaceTabs],
  );

  const reorderTab = useCallback(
    (sourceId: string, targetId: string) => {
      if (sourceId === targetId) return;
      setWorkspaceTabs((prev) => {
        // Reorder in display space (pinned-first): splicing the raw
        // stored array by raw indices would reorder it with no visible
        // change when the drag crosses the pinned boundary — flipping
        // the persisted order (and close-neighbor selection) on every
        // dragover event.
        const display = orderTabs(prev);
        const sourceIndex = display.findIndex((tab) => tab.id === sourceId);
        const targetIndex = display.findIndex((tab) => tab.id === targetId);
        if (sourceIndex === -1 || targetIndex === -1) return prev;
        if (
          Boolean(display[sourceIndex].pinned) !==
          Boolean(display[targetIndex].pinned)
        ) {
          return prev;
        }
        const next = [...display];
        const [moved] = next.splice(sourceIndex, 1);
        next.splice(targetIndex, 0, moved);
        return next;
      });
    },
    [setWorkspaceTabs],
  );

  // Roving tabindex: arrows move focus between tabs; Enter/Space
  // activates. Only the active tab is in the page tab order.
  const handleStripKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      const strip = tabsScrollerRef.current;
      if (!strip) return;
      const tabEls = [
        ...strip.querySelectorAll<HTMLElement>('[data-slot="workspace-tab"]'),
      ];
      const currentIndex = tabEls.findIndex(
        (el) => el === document.activeElement,
      );
      if (currentIndex === -1) return;
      event.preventDefault();
      const delta = event.key === "ArrowLeft" ? -1 : 1;
      const next =
        tabEls[(currentIndex + delta + tabEls.length) % tabEls.length];
      next?.focus();
    },
    [],
  );

  if (!hasWorkspaceTabs) {
    return null;
  }

  const activeEnvironment = (() => {
    const active = tabs.find((tab) => tab.id === activeTabId);
    if (!active) return undefined;
    return environmentById.get(active.connectionId);
  })();
  const envUnderline =
    activeEnvironment && activeEnvironment in ENV_UNDERLINE
      ? // SAFETY: the `in` check above proves the key is one of ENV_UNDERLINE's own keys.
        ENV_UNDERLINE[activeEnvironment as keyof typeof ENV_UNDERLINE]
      : undefined;

  return (
    <div
      data-slot="workspace-tabs"
      className={cn(
        "relative flex h-(--h-tab) shrink-0 items-stretch border-b border-border-subtle bg-surface-app",
        className,
      )}
    >
      <div
        ref={tabsScrollerRef}
        role="tablist"
        tabIndex={-1}
        aria-label="Open objects"
        data-testid="workspace-tabs-scroll"
        className="flex min-w-0 flex-1 items-stretch overflow-x-auto [scrollbar-width:none]"
        onScroll={recomputeOverflow}
        onKeyDown={handleStripKeyDown}
        onWheel={(event) => {
          // Mouse wheel scrolls the strip horizontally (§4.4).
          const scroller = tabsScrollerRef.current;
          if (!scroller || event.deltaY === 0 || event.deltaX !== 0) return;
          scroller.scrollLeft += event.deltaY;
        }}
      >
        {tabs.map((tab) => (
          <ObjectTab
            key={tab.id}
            tab={tab}
            isActive={tab.id === activeTabId}
            onActivate={() => setActiveTabId(tab.id)}
            onClose={() => requestClose(tab)}
            onCloseOthers={() =>
              closeMany(tabs.filter((other) => other.id !== tab.id))
            }
            onCloseToRight={() =>
              closeMany(tabs.slice(tabs.findIndex((t) => t.id === tab.id) + 1))
            }
            onCloseAll={() => closeMany(tabs)}
            onTogglePinned={() => togglePinned(tab.id)}
            onDragStartTab={() => {
              dragTabId.current = tab.id;
            }}
            onDragOverTab={(event) => {
              const sourceId = dragTabId.current;
              if (!sourceId || sourceId === tab.id) return;
              const sourceIndex = tabs.findIndex((t) => t.id === sourceId);
              const targetIndex = tabs.findIndex((t) => t.id === tab.id);
              if (sourceIndex === -1 || targetIndex === -1) return;
              // Only reorder once the pointer crosses the target's
              // midpoint in the drag direction — dragover fires
              // continuously, and with unequal tab widths an
              // unconditional swap oscillates under the pointer.
              const rect = event.currentTarget.getBoundingClientRect();
              const beforeMidpoint = event.clientX < rect.left + rect.width / 2;
              if (sourceIndex < targetIndex && beforeMidpoint) return;
              if (sourceIndex > targetIndex && !beforeMidpoint) return;
              reorderTab(sourceId, tab.id);
            }}
            onDragEndTab={() => {
              dragTabId.current = null;
            }}
          />
        ))}
      </div>
      {envUnderline ? (
        <div
          aria-hidden="true"
          data-testid="workspace-tabs-env-underline"
          className={cn(
            "pointer-events-none absolute inset-x-0 bottom-0 h-0.5",
            envUnderline,
          )}
        />
      ) : null}
      <div className="flex items-center gap-2 px-2">
        {overflowing ? <TabOverflowMenu tabs={tabs} /> : null}
        {sectionControl}
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                size="icon-sm"
                variant="ghost"
                aria-label="New query tab"
                onClick={createNewQueryTab}
              />
            }
          >
            <IconPlus />
          </TooltipTrigger>
          <TooltipContent>New query tab</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}

/** Trailing chevron listing every open tab when the strip overflows. */
function TabOverflowMenu({ tabs }: { tabs: WorkspaceTab[] }) {
  const { activeTabId, setActiveTabId } = useAppStore();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            size="icon-sm"
            variant="ghost"
            aria-label="All tabs"
          />
        }
      >
        <IconChevronDown />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-64">
        {tabs.map((tab) => {
          return (
            <DropdownMenuItem
              key={tab.id}
              className={cn(tab.id === activeTabId && "bg-accent-subdued")}
              onClick={() => setActiveTabId(tab.id)}
            >
              <WorkspaceTabIcon tab={tab} className="text-text-disabled" />
              <span className="truncate">{tab.label}</span>
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

function ObjectTab({
  tab,
  isActive,
  onActivate,
  onClose,
  onCloseOthers,
  onCloseToRight,
  onCloseAll,
  onTogglePinned,
  onDragStartTab,
  onDragOverTab,
  onDragEndTab,
}: {
  tab: WorkspaceTab;
  isActive: boolean;
  onActivate: () => void;
  onClose: () => void;
  onCloseOthers: () => void;
  onCloseToRight: () => void;
  onCloseAll: () => void;
  onTogglePinned: () => void;
  onDragStartTab: () => void;
  onDragOverTab: (event: React.DragEvent<HTMLDivElement>) => void;
  onDragEndTab: () => void;
}) {
  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            data-slot="workspace-tab"
            role="tab"
            aria-label={tab.label}
            aria-selected={isActive}
            tabIndex={isActive ? 0 : -1}
            draggable
            onDragStart={onDragStartTab}
            onDragOver={(event) => {
              event.preventDefault();
              onDragOverTab(event);
            }}
            onDragEnd={onDragEndTab}
            onClick={onActivate}
            onAuxClick={(event) => {
              // Middle-click closes (§4.4); pinned tabs are exempt.
              if (event.button === 1 && !tab.pinned) onClose();
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onActivate();
              }
            }}
            className={cn(
              "group/tab relative flex cursor-pointer items-center gap-1.5 border-r border-border-subtle text-sm transition-colors",
              tab.pinned ? "px-2" : "px-3",
              isActive
                ? "bg-surface-panel text-foreground"
                : "text-text-muted hover:bg-surface-panel/50 hover:text-foreground",
            )}
          />
        }
      >
        {isActive ? (
          // 2px accent top indicator (§4.4).
          <span
            aria-hidden="true"
            className="absolute inset-x-0 top-0 h-0.5 bg-accent"
          />
        ) : null}
        <WorkspaceTabIcon
          tab={tab}
          className="size-4 shrink-0 text-text-disabled"
        />
        {tab.pinned ? null : (
          <span className="max-w-40 truncate">{tab.label}</span>
        )}
        {tab.pinned ? null : (
          <span className="relative flex size-4 shrink-0 items-center justify-center">
            {/* Dirty dot yields to the close × on hover (§4.4). */}
            {tab.isDirty ? (
              <span
                data-testid={`workspace-tab-dirty-${tab.id}`}
                className="size-1.5 rounded-full bg-accent group-hover/tab:hidden"
              />
            ) : null}
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              aria-label={`Close ${tab.label}`}
              onClick={(event) => {
                event.stopPropagation();
                onClose();
              }}
              className={cn(
                "absolute inset-0 size-4 rounded-sm",
                tab.isDirty
                  ? "hidden group-hover/tab:inline-flex"
                  : "opacity-0 group-hover/tab:opacity-100 focus-visible:opacity-100",
              )}
            >
              <span aria-hidden="true">×</span>
            </Button>
          </span>
        )}
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem disabled={tab.pinned} onClick={onClose}>
          Close
        </ContextMenuItem>
        <ContextMenuItem onClick={onCloseOthers}>Close Others</ContextMenuItem>
        <ContextMenuItem onClick={onCloseToRight}>
          Close to the Right
        </ContextMenuItem>
        <ContextMenuItem onClick={onCloseAll}>Close All</ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onClick={onTogglePinned}>
          {tab.pinned ? "Unpin" : "Pin"}
        </ContextMenuItem>
        <ContextMenuItem
          onClick={() => {
            void navigator.clipboard.writeText(tab.label);
          }}
        >
          Copy Name
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

export function TableSectionToggle({
  value,
  onChange,
  showTransfer = false,
}: {
  value: TableSection;
  onChange: (next: TableSection) => void;
  showTransfer?: boolean;
}) {
  return (
    <Segmented<TableSection>
      value={value}
      onChange={onChange}
      options={[
        { id: "data", label: "Data", icon: <IconTable /> },
        { id: "schema", label: "Columns", icon: <IconColumns3 /> },
        { id: "indexes", label: "Keys", icon: <IconKey /> },
        { id: "relations", label: "Relations" },
        { id: "schema-map", label: "Schema Map" },
        ...(showTransfer
          ? ([{ id: "transfer", label: "Transfer" }] as const)
          : []),
        { id: "specialized", label: "Specialized" },
      ]}
    />
  );
}

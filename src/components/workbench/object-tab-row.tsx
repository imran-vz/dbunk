import {
  IconColumns3,
  IconKey,
  IconPlus,
  IconTable,
  IconTerminal2,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Segmented } from "@/components/ui/segmented";
import { ObjectTabCloseButton } from "@/components/workbench/dock";
import { confirmCloseQuerySession } from "@/lib/query-session-close";
import { useAppStore, type WorkspaceTab } from "@/lib/store";
import { cn } from "@/lib/utils";

const SCROLL_HINT_THRESHOLD_PX = 2;

export type TableSection = "data" | "schema" | "indexes";

interface ObjectTabRowProps {
  sectionControl?: React.ReactNode;
  className?: string;
}

export function ObjectTabRow({ sectionControl, className }: ObjectTabRowProps) {
  const tabsScrollerRef = useRef<HTMLDivElement>(null);
  const maxScrollLeftRef = useRef(0);
  const [scrollHints, setScrollHints] = useState({
    start: false,
    end: false,
  });
  const connections = useAppStore((state) => state.connections);
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const setActiveTabId = useAppStore((state) => state.setActiveTabId);
  const closeTab = useAppStore((state) => state.closeTab);
  const createNewQueryTab = useAppStore((state) => state.createNewQueryTab);
  const workspaceTabCount = workspaceTabs.length;
  const hasWorkspaceTabs = workspaceTabCount > 0;
  const productionConnectionIds = useMemo(
    () =>
      new Set(
        connections
          .filter((connection) => connection.environment === "production")
          .map((connection) => connection.id),
      ),
    [connections],
  );

  const updateScrollHints = useCallback(() => {
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    const maxScrollLeft = maxScrollLeftRef.current;
    const nextHints = {
      start: scroller.scrollLeft > SCROLL_HINT_THRESHOLD_PX,
      end: maxScrollLeft - scroller.scrollLeft > SCROLL_HINT_THRESHOLD_PX,
    };
    setScrollHints((currentHints) =>
      currentHints.start === nextHints.start &&
      currentHints.end === nextHints.end
        ? currentHints
        : nextHints,
    );
  }, []);

  const recomputeMaxScroll = useCallback(() => {
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    maxScrollLeftRef.current = Math.max(
      0,
      scroller.scrollWidth - scroller.clientWidth,
    );
    updateScrollHints();
  }, [updateScrollHints]);

  useEffect(() => {
    if (!hasWorkspaceTabs) return;
    recomputeMaxScroll();
    const scroller = tabsScrollerRef.current;
    if (!scroller) return;
    const resizeObserver =
      // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(recomputeMaxScroll);
    resizeObserver?.observe(scroller);
    window.addEventListener("resize", recomputeMaxScroll);
    return () => {
      resizeObserver?.disconnect();
      window.removeEventListener("resize", recomputeMaxScroll);
    };
  }, [hasWorkspaceTabs, recomputeMaxScroll]);

  if (!hasWorkspaceTabs) {
    return null;
  }

  return (
    <div
      data-slot="workspace-tabs"
      className={cn(
        "flex h-9 shrink-0 items-stretch border-b border-border-subtle bg-surface-app",
        className,
      )}
    >
      <div className="relative min-w-0 flex-1">
        <div
          ref={tabsScrollerRef}
          data-testid="workspace-tabs-scroll"
          className="flex min-w-0 items-stretch overflow-x-auto"
          onScroll={updateScrollHints}
        >
          {workspaceTabs.map((tab) => (
            <ObjectTab
              key={tab.id}
              tab={tab}
              isActive={tab.id === activeTabId}
              isProduction={productionConnectionIds.has(tab.connectionId)}
              onActivate={() => setActiveTabId(tab.id)}
              onClose={() => {
                const status =
                  useAppStore.getState().querySessions[tab.id]?.transaction
                    .status;
                if (!confirmCloseQuerySession(status)) return;
                void closeTab(tab.id);
              }}
            />
          ))}
        </div>
        {scrollHints.start ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-y-0 left-0 w-5 bg-linear-to-r from-surface-app to-transparent"
          />
        ) : null}
        {scrollHints.end ? (
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-y-0 right-0 w-5 bg-linear-to-l from-surface-app to-transparent"
          />
        ) : null}
      </div>
      <div className="flex items-center gap-2 px-2">
        {sectionControl}
        <button
          type="button"
          aria-label="New query tab"
          title="New query tab"
          onClick={createNewQueryTab}
          className="flex size-7 items-center justify-center rounded-md text-text-muted hover:bg-surface-panel hover:text-foreground"
        >
          <IconPlus className="size-3.5" />
        </button>
      </div>
    </div>
  );
}

function ObjectTab({
  tab,
  isActive,
  isProduction,
  onActivate,
  onClose,
}: {
  tab: WorkspaceTab;
  isActive: boolean;
  isProduction: boolean;
  onActivate: () => void;
  onClose: () => void;
}) {
  const TabIcon = tab.kind === "query" ? IconTerminal2 : IconTable;
  return (
    <div
      data-slot="workspace-tab"
      role="tab"
      aria-selected={isActive}
      tabIndex={0}
      onClick={onActivate}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onActivate();
        }
      }}
      className={cn(
        "flex cursor-pointer items-center gap-2 border-r border-border-subtle px-3 text-xs font-medium transition-colors",
        isActive
          ? "border-b-2 border-b-accent bg-surface-panel text-foreground"
          : "border-b-2 border-b-transparent text-text-muted hover:text-foreground",
        isActive && isProduction && "border-l-2 border-l-danger",
      )}
    >
      <TabIcon className="size-3.5 text-text-disabled" />
      <span className="max-w-40 truncate">{tab.label}</span>
      {tab.isDirty ? (
        <span
          data-testid={`workspace-tab-dirty-${tab.id}`}
          className="size-1.5 shrink-0 rounded-full bg-accent"
        />
      ) : null}
      <ObjectTabCloseButton label={tab.label} onClose={onClose} />
    </div>
  );
}

export function TableSectionToggle({
  value,
  onChange,
}: {
  value: TableSection;
  onChange: (next: TableSection) => void;
}) {
  return (
    <Segmented<TableSection>
      value={value}
      onChange={onChange}
      options={[
        {
          id: "data",
          label: "Data",
          icon: <IconTable className="size-3.5" />,
        },
        {
          id: "schema",
          label: "Columns",
          icon: <IconColumns3 className="size-3.5" />,
        },
        {
          id: "indexes",
          label: "Keys",
          icon: <IconKey className="size-3.5" />,
        },
      ]}
    />
  );
}

export function QuerySectionToggle({
  value,
  onChange,
}: {
  value: "results" | "explain" | "output";
  onChange: (next: "results" | "explain" | "output") => void;
}) {
  return (
    <Segmented<"results" | "explain" | "output">
      value={value}
      onChange={onChange}
      options={[
        { id: "results", label: "Results" },
        { id: "explain", label: "Explain" },
        { id: "output", label: "Output" },
      ]}
    />
  );
}

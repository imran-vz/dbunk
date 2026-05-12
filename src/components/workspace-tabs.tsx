import {
  IconChevronDown,
  IconPlus,
  IconTable,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

const SCROLL_HINT_THRESHOLD_PX = 2;

export function WorkspaceTabs() {
  const tabsScrollerRef = useRef<HTMLDivElement>(null);
  const maxScrollLeftRef = useRef(0);
  const [scrollHints, setScrollHints] = useState({
    start: false,
    end: false,
  });
  const {
    workspaceTabs,
    activeTabId,
    setActiveTabId,
    closeTab,
    createNewQueryTab,
  } = useAppStore();
  const workspaceTabCount = workspaceTabs.length;
  const hasWorkspaceTabs = workspaceTabCount > 0;

  const updateScrollHints = useCallback(() => {
    const scroller = tabsScrollerRef.current;
    if (!scroller) {
      return;
    }

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
    if (!scroller) {
      return;
    }
    maxScrollLeftRef.current = Math.max(
      0,
      scroller.scrollWidth - scroller.clientWidth,
    );
    updateScrollHints();
  }, [updateScrollHints]);

  useEffect(() => {
    if (!hasWorkspaceTabs) {
      return;
    }

    recomputeMaxScroll();

    const scroller = tabsScrollerRef.current;
    if (!scroller) {
      return;
    }

    const resizeObserver =
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

  const prevTabCountRef = useRef(workspaceTabCount);
  useEffect(() => {
    const prev = prevTabCountRef.current;
    prevTabCountRef.current = workspaceTabCount;

    if (workspaceTabCount === 0) {
      setScrollHints((currentHints) =>
        currentHints.start || currentHints.end
          ? { start: false, end: false }
          : currentHints,
      );
      return;
    }

    if (prev === 0) {
      return;
    }

    recomputeMaxScroll();
  }, [workspaceTabCount, recomputeMaxScroll]);

  if (!hasWorkspaceTabs) {
    return null;
  }

  return (
    <div
      data-slot="workspace-tabs"
      className="flex h-9 items-end border-b border-white/8 bg-[#0a0f14] px-3"
    >
      <div className="relative min-w-0 flex-1">
        <div
          ref={tabsScrollerRef}
          data-testid="workspace-tabs-scroll"
          className="flex min-w-0 items-end overflow-x-auto pr-1"
          onScroll={updateScrollHints}
        >
          {workspaceTabs.map((tab) => {
            const isActive = tab.id === activeTabId;
            const TabIcon = tab.kind === "query" ? IconTerminal2 : IconTable;
            return (
              <div
                key={tab.id}
                data-slot="workspace-tab"
                className={cn(
                  "group flex h-8 min-w-36 items-center gap-1.5 border border-b-0 px-2 text-[0.6875rem] transition",
                  isActive
                    ? "rounded-t-md border-white/12 bg-white/7.5 text-foreground"
                    : "border-transparent text-muted-foreground hover:bg-white/[0.035] hover:text-foreground",
                )}
              >
                <Button
                  size="xs"
                  variant="ghost"
                  className="min-w-0 flex-1 justify-start border-0 bg-transparent px-0 shadow-none hover:bg-transparent"
                  onClick={() => setActiveTabId(tab.id)}
                >
                  <TabIcon
                    className={cn(
                      "size-3.5",
                      isActive ? "text-primary" : "text-muted-foreground",
                    )}
                  />
                  <span className="truncate font-medium">{tab.label}</span>
                  {tab.isDirty ? (
                    <span
                      data-testid={`workspace-tab-dirty-${tab.id}`}
                      className="size-1.5 shrink-0 rounded-full bg-primary"
                    />
                  ) : null}
                </Button>
                <Button
                  size="icon-xs"
                  variant="ghost"
                  aria-label={`Close ${tab.label}`}
                  onClick={() => closeTab(tab.id)}
                  className="opacity-70 hover:opacity-100"
                >
                  <IconX />
                </Button>
              </div>
            );
          })}
        </div>
        {scrollHints.start ? (
          <div
            aria-hidden="true"
            data-testid="workspace-tabs-more-start"
            className="pointer-events-none absolute inset-y-0 left-0 w-5 bg-[linear-gradient(90deg,#0a0f14_35%,rgba(10,15,20,0))] after:absolute after:top-1 after:left-0 after:h-[calc(100%-0.5rem)] after:w-px after:bg-primary/60 after:shadow-[0_0_12px_rgba(155,230,74,0.42)]"
          />
        ) : null}
        {scrollHints.end ? (
          <div
            aria-hidden="true"
            data-testid="workspace-tabs-more-end"
            className="pointer-events-none absolute inset-y-0 right-0 w-6 bg-[linear-gradient(90deg,rgba(10,15,20,0),#0a0f14_68%)] after:absolute after:top-1 after:right-0 after:h-[calc(100%-0.5rem)] after:w-px after:bg-primary/70 after:shadow-[0_0_14px_rgba(155,230,74,0.48)]"
          />
        ) : null}
      </div>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label="New query tab"
        onClick={createNewQueryTab}
        className="mb-1"
      >
        <IconPlus className="size-3" />
      </Button>
      <Button
        size="icon-xs"
        variant="ghost"
        aria-label="Open tab menu"
        className="mb-1"
      >
        <IconChevronDown />
      </Button>
    </div>
  );
}

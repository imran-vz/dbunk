/**
 * Tab-management keyboard layer (DESIGN-SYSTEM §6.1, P7): `Cmd+T` new
 * query tab, `Cmd+W` close (skips pinned, confirms open transactions),
 * `Cmd+1..9` tab by visual position (9 = last), `Cmd+Shift+[`/`]`
 * previous/next tab, `Cmd+,` settings, and the `Ctrl+Tab` MRU switcher
 * popup (hold Ctrl, Tab cycles most-recent-first, release commits).
 */

import { IconEye, IconTable, IconTerminal2 } from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { confirmCloseQuerySession } from "@/lib/query-session-close";
import { useShortcutHandler } from "@/lib/shortcuts";
import { useAppStore, type WorkspaceTab } from "@/lib/store";
import { cn } from "@/lib/utils";

/** Visual order — pinned tabs leftmost, matching the object tab strip. */
function orderTabs(tabs: WorkspaceTab[]): WorkspaceTab[] {
  return [
    ...tabs.filter((tab) => tab.pinned),
    ...tabs.filter((tab) => !tab.pinned),
  ];
}

function WorkspaceTabIcon({ tab }: { tab: WorkspaceTab }) {
  const className = "size-3.5 shrink-0 text-text-muted";
  if (tab.kind === "query") return <IconTerminal2 className={className} />;
  if (tab.kind === "object") return <IconEye className={className} />;
  return <IconTable className={className} />;
}

async function closeActiveTab(): Promise<void> {
  const state = useAppStore.getState();
  const tab = state.workspaceTabs.find((item) => item.id === state.activeTabId);
  // §6.1: Cmd+W skips pinned tabs.
  if (!tab || tab.pinned) return;
  const status = state.querySessions[tab.id]?.transaction.status;
  if (!(await confirmCloseQuerySession(status))) return;
  void state.closeTab(tab.id);
}

export function TabShortcuts() {
  const workspaceTabs = useAppStore((state) => state.workspaceTabs);
  const activeTabId = useAppStore((state) => state.activeTabId);
  const createNewQueryTab = useAppStore((state) => state.createNewQueryTab);
  const openSettings = useAppStore((state) => state.openSettings);
  const setActiveView = useAppStore((state) => state.setActiveView);

  const orderedTabs = useMemo(() => orderTabs(workspaceTabs), [workspaceTabs]);
  const orderedRef = useRef(orderedTabs);
  orderedRef.current = orderedTabs;

  // Most-recent-first tab ids, maintained from activations and pruned
  // as tabs close.
  const mruRef = useRef<string[]>([]);
  useEffect(() => {
    if (!activeTabId) return;
    mruRef.current = [
      activeTabId,
      ...mruRef.current.filter((id) => id !== activeTabId),
    ];
  }, [activeTabId]);
  useEffect(() => {
    const live = new Set(workspaceTabs.map((tab) => tab.id));
    mruRef.current = mruRef.current.filter((id) => live.has(id));
  }, [workspaceTabs]);

  const [switcher, setSwitcher] = useState<{
    ids: string[];
    index: number;
  } | null>(null);
  const switcherRef = useRef(switcher);
  switcherRef.current = switcher;

  const openSettingsView = useCallback(() => {
    openSettings();
    setActiveView("settings");
  }, [openSettings, setActiveView]);

  const stepTab = useCallback((delta: number) => {
    const tabs = orderedRef.current;
    if (tabs.length < 2) return;
    const state = useAppStore.getState();
    const index = tabs.findIndex((tab) => tab.id === state.activeTabId);
    const next = tabs[(index + delta + tabs.length) % tabs.length];
    if (next) state.setActiveTabId(next.id);
  }, []);

  const goToIndex = useCallback((digit: number) => {
    const tabs = orderedRef.current;
    if (tabs.length === 0) return;
    // §6.1: 9 = last tab.
    const target = digit === 9 ? tabs[tabs.length - 1] : tabs[digit - 1];
    if (target) useAppStore.getState().setActiveTabId(target.id);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      // Ctrl+Tab MRU switcher (Shift reverses).
      if (event.ctrlKey && !event.metaKey && event.key === "Tab") {
        event.preventDefault();
        setSwitcher((current) => {
          const ids = current?.ids ?? mruRef.current;
          if (ids.length < 2) return current;
          const delta = event.shiftKey ? -1 : 1;
          const index = current
            ? (current.index + delta + ids.length) % ids.length
            : event.shiftKey
              ? ids.length - 1
              : 1;
          return { ids, index };
        });
        return;
      }
      const mod = event.metaKey || event.ctrlKey;
      if (!mod || event.altKey) return;
      const key = event.key.toLowerCase();
      if (!event.shiftKey && key === "t") {
        event.preventDefault();
        createNewQueryTab();
      } else if (!event.shiftKey && key === "w") {
        event.preventDefault();
        void closeActiveTab();
      } else if (key === ",") {
        event.preventDefault();
        openSettingsView();
      } else if (!event.shiftKey && /^[1-9]$/.test(event.key)) {
        event.preventDefault();
        goToIndex(Number(event.key));
      } else if (event.shiftKey && event.code === "BracketLeft") {
        event.preventDefault();
        stepTab(-1);
      } else if (event.shiftKey && event.code === "BracketRight") {
        event.preventDefault();
        stepTab(1);
      }
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.key !== "Control") return;
      const current = switcherRef.current;
      if (!current) return;
      const id = current.ids[current.index];
      setSwitcher(null);
      if (id) useAppStore.getState().setActiveTabId(id);
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [createNewQueryTab, goToIndex, openSettingsView, stepTab]);

  // Palette handlers for the tab commands (§4.10).
  useShortcutHandler("new-query-tab", createNewQueryTab);
  useShortcutHandler(
    "close-tab",
    useCallback(() => void closeActiveTab(), []),
  );
  useShortcutHandler("settings", openSettingsView);
  useShortcutHandler(
    "prev-tab",
    useCallback(() => stepTab(-1), [stepTab]),
  );
  useShortcutHandler(
    "next-tab",
    useCallback(() => stepTab(1), [stepTab]),
  );

  if (!switcher) return null;
  const tabsById = new Map(workspaceTabs.map((tab) => [tab.id, tab]));

  return (
    <div className="pointer-events-none fixed inset-0 z-50 flex items-start justify-center pt-[20vh]">
      <div
        data-testid="mru-tab-switcher"
        className="pointer-events-auto w-80 overflow-hidden rounded-md border border-border-subtle bg-surface-panel-elevated shadow-xl"
      >
        <ul className="max-h-72 overflow-auto p-1">
          {switcher.ids.map((id, index) => {
            const tab = tabsById.get(id);
            if (!tab) return null;
            const active = index === switcher.index;
            return (
              <li
                key={id}
                data-active={active || undefined}
                className={cn(
                  "flex items-center gap-2 rounded-sm px-2 py-1.5 text-xs",
                  active
                    ? "bg-accent-subdued text-foreground"
                    : "text-text-secondary",
                )}
              >
                <WorkspaceTabIcon tab={tab} />
                <span className="truncate">{tab.label}</span>
              </li>
            );
          })}
        </ul>
      </div>
    </div>
  );
}

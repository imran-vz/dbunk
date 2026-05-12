import {
  IconChevronDown,
  IconPlus,
  IconTable,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { Button } from "@/components/ui/button";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export function WorkspaceTabs() {
  const {
    workspaceTabs,
    activeTabId,
    setActiveTabId,
    closeTab,
    createNewQueryTab,
  } = useAppStore();

  if (workspaceTabs.length === 0) {
    return null;
  }

  return (
    <div
      data-slot="workspace-tabs"
      className="flex h-12 items-end border-b border-white/8 bg-[#0a0f14] px-5"
    >
      <div className="flex min-w-0 flex-1 items-end overflow-x-auto">
        {workspaceTabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const TabIcon = tab.kind === "query" ? IconTerminal2 : IconTable;
          return (
            <div
              key={tab.id}
              className={cn(
                "group flex h-10 min-w-42 items-center gap-2 border border-b-0 px-3 text-xs transition",
                isActive
                  ? "rounded-t-lg border-white/12 bg-white/[0.075] text-foreground"
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
                  <span className="size-1.5 shrink-0 rounded-full bg-primary" />
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
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="New query tab"
        onClick={createNewQueryTab}
        className="mb-1"
      >
        <IconPlus className="size-3.5" />
      </Button>
      <Button
        size="icon-sm"
        variant="ghost"
        aria-label="Open tab menu"
        className="mb-1"
      >
        <IconChevronDown />
      </Button>
    </div>
  );
}

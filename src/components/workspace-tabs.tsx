import {
  IconChevronDown,
  IconTable,
  IconTerminal2,
  IconX,
} from "@tabler/icons-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useAppStore } from "@/lib/store";
import { cn } from "@/lib/utils";

export function WorkspaceTabs() {
  const { workspaceTabs, activeTabId, setActiveTabId, closeTab } =
    useAppStore();

  return (
    <div className="flex items-center gap-2 border-b px-4 py-2">
      <div className="flex min-w-0 flex-1 items-center gap-2 overflow-x-auto">
        {workspaceTabs.map((tab) => {
          const isActive = tab.id === activeTabId;
          const TabIcon = tab.kind === "query" ? IconTerminal2 : IconTable;
          return (
            <div
              key={tab.id}
              className={cn(
                "flex items-center gap-1 rounded-md border px-1 py-1 text-xs",
                isActive
                  ? "border-border bg-muted"
                  : "border-transparent bg-transparent hover:border-border hover:bg-muted/40",
              )}
            >
              <Button
                size="sm"
                variant="ghost"
                className="h-6 gap-2 px-2 text-xs"
                onClick={() => setActiveTabId(tab.id)}
              >
                <TabIcon className="size-3.5" />
                <span className="max-w-35 truncate">{tab.label}</span>
                {tab.isDirty ? (
                  <span className="size-1.5 rounded-full bg-primary" />
                ) : null}
                <Badge variant="secondary" className="text-[0.625rem]">
                  {tab.kind === "query" ? "Query" : "Table"}
                </Badge>
              </Button>
              <Button
                size="icon-xs"
                variant="ghost"
                aria-label={`Close ${tab.label}`}
                onClick={() => closeTab(tab.id)}
              >
                <IconX />
              </Button>
            </div>
          );
        })}
      </div>
      <Button size="icon-sm" variant="ghost" aria-label="Open tab menu">
        <IconChevronDown />
      </Button>
    </div>
  );
}

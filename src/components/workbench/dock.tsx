import {
  IconChevronDown,
  IconChevronUp,
  IconColumns3,
  IconPlayerPlayFilled,
  IconTerminal,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export type DockTabId = "console" | "output";

interface WorkbenchDockProps {
  consoleLabel?: string;
  consoleContent: React.ReactNode;
  outputContent: React.ReactNode;
  onRun?: () => void;
  runDisabled?: boolean;
  storageKey?: string;
  className?: string;
}

const DOCK_STORAGE_PREFIX = "dbunk.workbench.dock.";

function DockTabButton({
  active,
  onClick,
  icon,
  label,
}: {
  active: boolean;
  onClick: () => void;
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex items-center gap-1.5 rounded px-2.5 py-1 text-[11px] font-medium transition-colors",
        active
          ? "bg-accent-subdued text-accent"
          : "text-text-muted hover:text-foreground",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

export function WorkbenchDock({
  consoleLabel = "scratch.sql",
  consoleContent,
  outputContent,
  onRun,
  runDisabled,
  storageKey = "default",
  className,
}: WorkbenchDockProps) {
  const storageId = `${DOCK_STORAGE_PREFIX}${storageKey}`;
  const [dockOpen, setDockOpen] = useState(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof window === "undefined") return true;
    return window.localStorage.getItem(`${storageId}.open`) !== "false";
  });
  const [dockTab, setDockTab] = useState<DockTabId>(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof window === "undefined") return "console";
    const saved = window.localStorage.getItem(`${storageId}.tab`);
    return saved === "output" ? "output" : "console";
  });

  useEffect(() => {
    window.localStorage.setItem(`${storageId}.open`, String(dockOpen));
  }, [dockOpen, storageId]);

  useEffect(() => {
    window.localStorage.setItem(`${storageId}.tab`, dockTab);
  }, [dockTab, storageId]);

  return (
    <div
      data-slot="workbench-dock"
      className={cn(
        "shrink-0 border-t border-border-subtle bg-surface-panel",
        className,
      )}
    >
      <div className="flex items-center gap-1 px-2 py-1">
        <DockTabButton
          active={dockOpen && dockTab === "console"}
          onClick={() => {
            setDockTab("console");
            setDockOpen(true);
          }}
          icon={<IconTerminal className="size-3.5" />}
          label="SQL Console"
        />
        <DockTabButton
          active={dockOpen && dockTab === "output"}
          onClick={() => {
            setDockTab("output");
            setDockOpen(true);
          }}
          icon={<IconColumns3 className="size-3.5" />}
          label="Output"
        />
        <button
          type="button"
          aria-label={dockOpen ? "Collapse dock" : "Expand dock"}
          onClick={() => setDockOpen((open) => !open)}
          className="ml-auto flex items-center gap-1 rounded px-2 py-1 text-[11px] text-text-muted hover:text-foreground"
        >
          {dockOpen ? (
            <IconChevronDown className="size-3.5" />
          ) : (
            <IconChevronUp className="size-3.5" />
          )}
        </button>
      </div>
      {dockOpen ? (
        <div
          className="h-40 overflow-auto border-t border-border-subtle"
          data-testid="workbench-dock-body"
        >
          {dockTab === "console" ? (
            <div className="flex h-full flex-col">
              <div className="flex items-center justify-between px-3 py-1.5">
                <span className="text-[11px] text-text-muted">
                  {consoleLabel}
                </span>
                {onRun ? (
                  <Button
                    type="button"
                    size="xs"
                    disabled={runDisabled}
                    onClick={onRun}
                    className="gap-1.5"
                  >
                    <IconPlayerPlayFilled className="size-3" />
                    Run
                  </Button>
                ) : null}
              </div>
              <div className="min-h-0 flex-1 overflow-auto">
                {consoleContent}
              </div>
            </div>
          ) : (
            <div className="min-h-0 flex-1 overflow-auto p-2">
              {outputContent}
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}

export function ObjectTabCloseButton({
  label,
  onClose,
}: {
  label: string;
  onClose: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={`Close ${label}`}
      onClick={(event) => {
        event.stopPropagation();
        onClose();
      }}
      className="rounded p-0.5 text-text-muted hover:text-foreground"
    >
      <IconX className="size-3" />
    </button>
  );
}

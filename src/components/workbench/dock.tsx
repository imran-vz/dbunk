import {
  IconChevronDown,
  IconChevronUp,
  IconColumns3,
  IconX,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

import { cn } from "@/lib/utils";

interface WorkbenchDockProps {
  content: React.ReactNode;
  /**
   * When this value changes the dock reveals itself. Wired to the active
   * results view so switching Results/Explain/Output (or running a query)
   * always produces a visible change even if the dock was collapsed.
   */
  revealKey?: unknown;
  storageKey?: string;
  className?: string;
}

const DOCK_STORAGE_PREFIX = "dbunk.workbench.dock.";

export function WorkbenchDock({
  content,
  revealKey,
  storageKey = "default",
  className,
}: WorkbenchDockProps) {
  const storageId = `${DOCK_STORAGE_PREFIX}${storageKey}`;
  const [dockOpen, setDockOpen] = useState(() => {
    // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
    if (typeof window === "undefined") return true;
    return window.localStorage.getItem(`${storageId}.open`) !== "false";
  });

  useEffect(() => {
    window.localStorage.setItem(`${storageId}.open`, String(dockOpen));
  }, [dockOpen, storageId]);

  const isFirstReveal = useRef(true);
  useEffect(() => {
    if (isFirstReveal.current) {
      isFirstReveal.current = false;
      return;
    }
    setDockOpen(true);
  }, [revealKey]);

  return (
    <div
      data-slot="workbench-dock"
      className={cn(
        "shrink-0 border-t border-border-subtle bg-surface-panel",
        className,
      )}
    >
      <div className="flex items-center gap-1 px-2 py-1">
        <button
          type="button"
          onClick={() => setDockOpen(true)}
          className={cn(
            "flex items-center gap-1.5 rounded px-2.5 py-1 text-2xs font-medium transition-colors",
            dockOpen
              ? "bg-accent-subdued text-accent"
              : "text-text-muted hover:text-foreground",
          )}
        >
          <IconColumns3 className="size-3.5" />
          Output
        </button>
        <button
          type="button"
          aria-label={dockOpen ? "Collapse dock" : "Expand dock"}
          onClick={() => setDockOpen((open) => !open)}
          className="ml-auto flex items-center gap-1 rounded px-2 py-1 text-2xs text-text-muted hover:text-foreground"
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
          <div className="min-h-0 flex-1 overflow-auto p-2">{content}</div>
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

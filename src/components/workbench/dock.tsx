import {
  IconChevronDown,
  IconChevronUp,
  IconColumns3,
} from "@tabler/icons-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "@/components/ui/button";
import { usePanelState } from "@/components/ui/panel";
import { Sash } from "@/components/ui/resizer-handle";
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
/** §3.3: dock max = 60% of content height. */
const DOCK_MAX = () => Math.round(window.innerHeight * 0.6);

/**
 * Output dock — interim surface until P4 replaces it with the global
 * console + editor/results split. Height is resizable on the shared
 * sash spec and persisted (§3.3: min 100 / default 200 / snap 60).
 */
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

  const height = usePanelState({
    storageKey: `${storageId}.panel`,
    defaultSize: 200,
    min: 100,
    max: DOCK_MAX,
    snapThreshold: 60,
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
        <Button
          type="button"
          size="xs"
          variant="ghost"
          onClick={() => setDockOpen(true)}
          className={cn(
            dockOpen && "bg-accent-subdued text-accent hover:bg-accent-subdued",
          )}
        >
          <IconColumns3 />
          Output
        </Button>
        <Button
          type="button"
          size="icon-xs"
          variant="ghost"
          aria-label={dockOpen ? "Collapse dock" : "Expand dock"}
          onClick={() => setDockOpen((open) => !open)}
          className="ml-auto"
        >
          {dockOpen ? <IconChevronDown /> : <IconChevronUp />}
        </Button>
      </div>
      {dockOpen ? (
        <div
          className="relative flex flex-col overflow-hidden border-t border-border-subtle"
          style={{ height: height.size }}
          data-testid="workbench-dock-body"
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
            ariaLabel="Resize output dock"
          />
          <div className="min-h-0 flex-1 overflow-auto p-2">{content}</div>
        </div>
      ) : null}
    </div>
  );
}

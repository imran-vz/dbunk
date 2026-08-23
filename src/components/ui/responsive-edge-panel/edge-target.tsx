import {
  IconLayoutSidebarLeftExpand,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";

import { cn } from "@/lib/utils";

import type { PanelSide } from "./use-panel";

interface EdgeTargetProps {
  side: PanelSide;
  label: string;
  panelId: string;
  onHover: () => void;
  onActivate: () => void;
}

export function EdgeTarget({
  side,
  label,
  panelId,
  onHover,
  onActivate,
}: EdgeTargetProps) {
  const ToggleIcon =
    side === "left"
      ? IconLayoutSidebarLeftExpand
      : IconLayoutSidebarRightExpand;
  return (
    <div
      className={cn(
        "absolute inset-y-0 z-30 hidden w-3 md:block",
        side === "left" ? "left-0" : "right-0",
      )}
    >
      <button
        type="button"
        aria-label={label}
        aria-controls={panelId}
        onMouseEnter={onHover}
        onClick={onActivate}
        className={cn(
          "absolute top-1/2 flex h-14 w-6 -translate-y-1/2 items-center justify-center border border-border-subtle bg-surface-window/95 text-text-muted shadow-lg hover:text-foreground",
          side === "left"
            ? "left-0 rounded-r-md border-l-0"
            : "right-0 rounded-l-md border-r-0",
        )}
      >
        <ToggleIcon className="size-3.5" />
      </button>
    </div>
  );
}

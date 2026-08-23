import {
  IconLayoutSidebarLeftCollapse,
  IconLayoutSidebarRightCollapse,
  IconPin,
  IconPinnedOff,
} from "@tabler/icons-react";
import type * as React from "react";

import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

import type { PanelSide } from "./use-panel";

interface PanelAsideProps {
  panelId: string;
  panelRef: React.Ref<HTMLDivElement>;
  side: PanelSide;
  title: string;
  width: number;
  isCompact: boolean;
  reserveSpace: boolean;
  pinned: boolean;
  requestedOpen: boolean;
  onTogglePinned: () => void;
  onClose: () => void;
  onMouseLeave: () => void;
  className?: string;
  contentClassName?: string;
  renderCompactHeader?: (props: {
    pinned: boolean;
    onTogglePinned: () => void;
    onClose: () => void;
  }) => React.ReactNode;
  children: React.ReactNode;
}

export function PanelAside({
  panelId,
  panelRef,
  side,
  title,
  width,
  isCompact,
  reserveSpace,
  pinned,
  requestedOpen,
  onTogglePinned,
  onClose,
  onMouseLeave,
  className,
  contentClassName,
  renderCompactHeader,
  children,
}: PanelAsideProps) {
  return (
    /* oxlint-disable-next-line jsx-a11y/no-noninteractive-element-interactions -- Pointer exit controls the transient aside; the panel itself is not an interactive control. */
    <aside
      id={panelId}
      ref={panelRef}
      style={{ width }}
      onMouseLeave={() => {
        if (!requestedOpen) onMouseLeave();
      }}
      className={cn(
        "z-40 flex min-h-0 shrink-0 flex-col border-border-subtle bg-surface-window text-xs text-foreground shadow-none",
        side === "left" ? "border-r" : "border-l",
        !reserveSpace &&
          cn(
            "absolute inset-y-0 shadow-2xl",
            side === "left" ? "left-0" : "right-0",
          ),
        className,
      )}
    >
      {isCompact
        ? (renderCompactHeader?.({
            pinned,
            onTogglePinned,
            onClose,
          }) ?? (
            <DefaultCompactHeader
              side={side}
              title={title}
              pinned={pinned}
              onTogglePinned={onTogglePinned}
              onClose={onClose}
            />
          ))
        : null}
      <div className={cn("min-h-0 flex-1", contentClassName)}>{children}</div>
    </aside>
  );
}

interface DefaultCompactHeaderProps {
  side: PanelSide;
  title: string;
  pinned: boolean;
  onTogglePinned: () => void;
  onClose: () => void;
}

function DefaultCompactHeader({
  side,
  title,
  pinned,
  onTogglePinned,
  onClose,
}: DefaultCompactHeaderProps) {
  const CollapseIcon =
    side === "left"
      ? IconLayoutSidebarLeftCollapse
      : IconLayoutSidebarRightCollapse;
  return (
    <div className="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-border-subtle px-2">
      <span className="truncate text-2xs font-semibold uppercase tracking-[0.12em] text-text-muted">
        {title}
      </span>
      <div className="flex items-center gap-0.5">
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                size="icon-xs"
                variant={pinned ? "secondary" : "ghost"}
                aria-pressed={pinned}
                aria-label={pinned ? `Unpin ${title}` : `Pin ${title}`}
                onClick={onTogglePinned}
              />
            }
          >
            {pinned ? <IconPinnedOff /> : <IconPin />}
          </TooltipTrigger>
          <TooltipContent>
            {pinned ? `Unpin ${title}` : `Pin ${title}`}
          </TooltipContent>
        </Tooltip>
        <Tooltip>
          <TooltipTrigger
            render={
              <Button
                type="button"
                size="icon-xs"
                variant="ghost"
                aria-label={`Hide ${title}`}
                onClick={onClose}
              />
            }
          >
            <CollapseIcon />
          </TooltipTrigger>
          <TooltipContent>{`Hide ${title}`}</TooltipContent>
        </Tooltip>
      </div>
    </div>
  );
}

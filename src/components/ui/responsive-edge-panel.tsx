import {
  IconLayoutSidebarLeftCollapse,
  IconLayoutSidebarLeftExpand,
  IconLayoutSidebarRightCollapse,
  IconLayoutSidebarRightExpand,
  IconPin,
  IconPinnedOff,
} from "@tabler/icons-react";
import type * as React from "react";
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";

import { Button } from "@/components/ui/button";
import { ResizerHandle } from "@/components/ui/resizer-handle";
import { cn } from "@/lib/utils";

type PanelSide = "left" | "right";

interface ResponsiveEdgePanelProps {
  side: PanelSide;
  storageKey: string;
  title: string;
  width: number;
  containerWidth: number;
  compactBelow?: number;
  protectedWorkspaceWidth?: number;
  defaultPinned?: boolean;
  wideVisible?: boolean;
  open?: boolean;
  defaultOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  resizer?: {
    onResize: (next: number) => void;
    min?: number;
    max?: number;
    ariaLabel?: string;
  };
  className?: string;
  contentClassName?: string;
  children: React.ReactNode;
}

const readStoredBool = (key: string, fallback: boolean) => {
  if (typeof window === "undefined") return fallback;
  const raw = window.localStorage.getItem(key);
  if (raw === null) return fallback;
  return raw === "1";
};

export function ResponsiveEdgePanel({
  side,
  storageKey,
  title,
  width,
  containerWidth,
  compactBelow = 1100,
  protectedWorkspaceWidth = 560,
  defaultPinned = false,
  wideVisible = true,
  open,
  defaultOpen = false,
  onOpenChange,
  resizer,
  className,
  contentClassName,
  children,
}: ResponsiveEdgePanelProps) {
  const panelId = useId();
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [pinned, setPinned] = useState(() =>
    readStoredBool(`${storageKey}.pinned`, defaultPinned),
  );
  const [uncontrolledOpen, setUncontrolledOpen] = useState(defaultOpen);
  const [hovered, setHovered] = useState(false);
  const requestedOpen = open ?? uncontrolledOpen;

  const isCompact = containerWidth > 0 ? containerWidth < compactBelow : false;
  const canReserve =
    containerWidth === 0 || containerWidth - width >= protectedWorkspaceWidth;
  const reserveSpace = !isCompact
    ? wideVisible
    : requestedOpen && pinned && canReserve;
  const overlayOpen = isCompact && !reserveSpace && (hovered || requestedOpen);
  const visible = reserveSpace || overlayOpen;
  const showEdgeTarget = isCompact && !reserveSpace && !overlayOpen;
  const showResizer = visible && reserveSpace && resizer;

  const setRequestedOpen = useCallback(
    (next: boolean) => {
      if (open === undefined) {
        setUncontrolledOpen(next);
      }
      onOpenChange?.(next);
    },
    [onOpenChange, open],
  );

  useEffect(() => {
    if (typeof window === "undefined") return;
    window.localStorage.setItem(`${storageKey}.pinned`, pinned ? "1" : "0");
  }, [pinned, storageKey]);

  useEffect(() => {
    if (!isCompact) {
      setHovered(false);
      setRequestedOpen(false);
    }
  }, [isCompact, setRequestedOpen]);

  useEffect(() => {
    if (!overlayOpen || canReserve) return;
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (panelRef.current?.contains(target)) return;
      setRequestedOpen(false);
      setHovered(false);
    };
    window.addEventListener("pointerdown", onPointerDown);
    return () => window.removeEventListener("pointerdown", onPointerDown);
  }, [canReserve, overlayOpen, setRequestedOpen]);

  const edgeLabel =
    side === "left" ? `Show ${title} panel` : `Show ${title} panel`;
  const ToggleIcon =
    side === "left"
      ? IconLayoutSidebarLeftExpand
      : IconLayoutSidebarRightExpand;
  const CollapseIcon =
    side === "left"
      ? IconLayoutSidebarLeftCollapse
      : IconLayoutSidebarRightCollapse;

  const panelStyle = useMemo<React.CSSProperties>(
    () => ({
      width,
    }),
    [width],
  );

  const panel = visible ? (
    <aside
      id={panelId}
      ref={panelRef}
      style={panelStyle}
      onMouseLeave={() => {
        if (!requestedOpen) setHovered(false);
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
      {isCompact ? (
        <div className="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-border-subtle px-2">
          <span className="truncate text-[0.65rem] font-semibold uppercase tracking-[0.12em] text-text-muted">
            {title}
          </span>
          <div className="flex items-center gap-0.5">
            <Button
              type="button"
              size="icon-xs"
              variant={pinned ? "secondary" : "ghost"}
              aria-pressed={pinned}
              aria-label={pinned ? `Unpin ${title}` : `Pin ${title}`}
              title={pinned ? `Unpin ${title}` : `Pin ${title}`}
              onClick={() => {
                setPinned((next) => !next);
                setRequestedOpen(true);
              }}
            >
              {pinned ? (
                <IconPinnedOff className="size-3" />
              ) : (
                <IconPin className="size-3" />
              )}
            </Button>
            <Button
              type="button"
              size="icon-xs"
              variant="ghost"
              aria-label={`Hide ${title}`}
              title={`Hide ${title}`}
              onClick={() => {
                setRequestedOpen(false);
                setHovered(false);
              }}
            >
              <CollapseIcon className="size-3" />
            </Button>
          </div>
        </div>
      ) : null}
      <div className={cn("min-h-0 flex-1", contentClassName)}>{children}</div>
    </aside>
  ) : null;

  const resizeHandle = showResizer ? (
    <ResizerHandle
      width={width}
      onResize={resizer.onResize}
      side={side === "left" ? "right" : "left"}
      min={resizer.min}
      max={resizer.max}
      ariaLabel={resizer.ariaLabel}
    />
  ) : null;

  return (
    <>
      {showEdgeTarget ? (
        <div
          className={cn(
            "absolute inset-y-0 z-30 hidden w-3 md:block",
            side === "left" ? "left-0" : "right-0",
          )}
        >
          <button
            type="button"
            aria-label={edgeLabel}
            aria-controls={panelId}
            title={edgeLabel}
            onMouseEnter={() => setHovered(true)}
            onClick={() => setRequestedOpen(true)}
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
      ) : null}

      {isCompact && !reserveSpace ? (
        <Button
          type="button"
          size="icon-xs"
          variant="secondary"
          aria-label={edgeLabel}
          aria-controls={panelId}
          title={edgeLabel}
          onClick={() => setRequestedOpen(!requestedOpen)}
          className={cn(
            "absolute top-2 z-40 shadow-lg md:hidden",
            side === "left" ? "left-2" : "right-2",
          )}
        >
          <ToggleIcon className="size-3" />
        </Button>
      ) : null}

      {side === "right" ? resizeHandle : null}
      {panel}
      {side === "left" ? resizeHandle : null}
    </>
  );
}

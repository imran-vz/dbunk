import type * as React from "react";

import { ResizerHandle } from "@/components/ui/resizer-handle";

import { CompactToggle } from "./responsive-edge-panel/compact-toggle";
import { EdgeTarget } from "./responsive-edge-panel/edge-target";
import { PanelAside } from "./responsive-edge-panel/panel-aside";
import { type PanelSide, usePanel } from "./responsive-edge-panel/use-panel";

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
  renderCompactHeader?: (props: {
    pinned: boolean;
    onTogglePinned: () => void;
    onClose: () => void;
  }) => React.ReactNode;
  children: React.ReactNode;
}

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
  renderCompactHeader,
  children,
}: ResponsiveEdgePanelProps) {
  const panel = usePanel({
    storageKey,
    width,
    containerWidth,
    compactBelow,
    protectedWorkspaceWidth,
    defaultPinned,
    wideVisible,
    open,
    defaultOpen,
    hasResizer: Boolean(resizer),
    onOpenChange,
  });

  const edgeLabel = `Show ${title} panel`;
  const openHandler = () => panel.setRequestedOpen(true);
  const toggleHandler = () => panel.setRequestedOpen(!panel.requestedOpen);

  const aside = panel.visible ? (
    <PanelAside
      panelId={panel.panelId}
      panelRef={panel.panelRef}
      side={side}
      title={title}
      width={width}
      isCompact={panel.isCompact}
      reserveSpace={panel.reserveSpace}
      pinned={panel.pinned}
      requestedOpen={panel.requestedOpen}
      onTogglePinned={panel.togglePinned}
      onClose={panel.closePanel}
      onMouseLeave={() => panel.setHovered(false)}
      className={className}
      contentClassName={contentClassName}
      renderCompactHeader={renderCompactHeader}
    >
      {children}
    </PanelAside>
  ) : null;

  const resizeHandle =
    panel.showResizer && resizer ? (
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
      {panel.showEdgeTarget ? (
        <EdgeTarget
          side={side}
          label={edgeLabel}
          panelId={panel.panelId}
          onHover={() => panel.setHovered(true)}
          onActivate={openHandler}
        />
      ) : null}

      {panel.isCompact && !panel.reserveSpace ? (
        <CompactToggle
          side={side}
          label={edgeLabel}
          panelId={panel.panelId}
          onActivate={toggleHandler}
        />
      ) : null}

      {side === "right" ? resizeHandle : null}
      {aside}
      {side === "left" ? resizeHandle : null}
    </>
  );
}

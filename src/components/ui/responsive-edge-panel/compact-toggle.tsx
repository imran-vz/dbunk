import {
  IconLayoutSidebarLeftExpand,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import type { PanelSide } from "./use-panel";

interface CompactToggleProps {
  side: PanelSide;
  label: string;
  panelId: string;
  onActivate: () => void;
}

export function CompactToggle({
  side,
  label,
  panelId,
  onActivate,
}: CompactToggleProps) {
  const ToggleIcon =
    side === "left"
      ? IconLayoutSidebarLeftExpand
      : IconLayoutSidebarRightExpand;
  return (
    <Button
      type="button"
      size="icon-xs"
      variant="secondary"
      aria-label={label}
      aria-controls={panelId}
      title={label}
      onClick={onActivate}
      className={cn(
        "absolute top-2 z-40 shadow-lg md:hidden",
        side === "left" ? "left-2" : "right-2",
      )}
    >
      <ToggleIcon className="size-3" />
    </Button>
  );
}

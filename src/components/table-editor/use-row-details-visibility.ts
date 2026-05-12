import { useState } from "react";

export const ROW_DETAILS_WIDTH = 320;
export const ROW_DETAILS_COMPACT_BELOW = 980;
export const PROTECTED_WORKSPACE_WIDTH = 560;

export interface RowDetailsVisibility {
  isOpen: boolean;
  overlayOpen: boolean;
  visible: boolean;
  isCompact: boolean;
  setOverlayOpen: (open: boolean) => void;
  onToggle: () => void;
  onClose: () => void;
}

export function useRowDetailsVisibility(
  bodyWidth: number,
): RowDetailsVisibility {
  const [isOpen, setIsOpen] = useState(true);
  const [overlayOpen, setOverlayOpen] = useState(false);

  const isCompact = bodyWidth > 0 && bodyWidth < ROW_DETAILS_COMPACT_BELOW;
  const visible = isCompact ? overlayOpen : isOpen;

  return {
    isOpen,
    overlayOpen,
    visible,
    isCompact,
    setOverlayOpen,
    onToggle: () => {
      if (isCompact) setOverlayOpen(!overlayOpen);
      else setIsOpen((open) => !open);
    },
    onClose: () => {
      if (isCompact) setOverlayOpen(false);
      else setIsOpen(false);
    },
  };
}

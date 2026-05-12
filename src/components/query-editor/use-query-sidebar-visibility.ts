import { useState } from "react";

export const QUERY_SIDEBAR_WIDTH = 304;
export const QUERY_SIDEBAR_COMPACT_BELOW = 1120;
export const PROTECTED_WORKSPACE_WIDTH = 560;

export interface QuerySidebarVisibility {
  wideVisible: boolean;
  overlayOpen: boolean;
  isOpen: boolean;
  isCompact: boolean;
  setOverlayOpen: (open: boolean) => void;
  onToggle: () => void;
}

export function useQuerySidebarVisibility(
  containerWidth: number,
): QuerySidebarVisibility {
  const [wideVisible, setWideVisible] = useState(true);
  const [overlayOpen, setOverlayOpen] = useState(false);

  const isCompact =
    containerWidth > 0 && containerWidth < QUERY_SIDEBAR_COMPACT_BELOW;
  const isOpen = isCompact ? overlayOpen : wideVisible;

  return {
    wideVisible,
    overlayOpen,
    isOpen,
    isCompact,
    setOverlayOpen,
    onToggle: () => {
      if (isCompact) setOverlayOpen(!overlayOpen);
      else setWideVisible((visible) => !visible);
    },
  };
}

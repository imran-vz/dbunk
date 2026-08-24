import { useState } from "react";

/**
 * Parent-controlled presence for the query details panel. Width,
 * resize, and sash-collapse mechanics live in the `Panel` primitive
 * (`usePanelState`); this hook only answers "is the panel structurally
 * present at all" for the toolbar toggle and close affordance.
 */
export interface QuerySidebarVisibility {
  isOpen: boolean;
  onToggle: () => void;
  onClose: () => void;
}

export function useQuerySidebarVisibility(): QuerySidebarVisibility {
  const [isOpen, setIsOpen] = useState(true);

  return {
    isOpen,
    onToggle: () => setIsOpen((open) => !open),
    onClose: () => setIsOpen(false),
  };
}

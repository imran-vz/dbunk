import { useState } from "react";

/**
 * Parent-controlled presence for the row-details panel. Width, resize,
 * and sash-collapse mechanics live in the `Panel` primitive
 * (`usePanelState`); this hook only answers "is the panel structurally
 * present at all" for the toolbar toggle and close affordance.
 */
export interface RowDetailsVisibility {
  visible: boolean;
  onToggle: () => void;
  onClose: () => void;
}

export function useRowDetailsVisibility(): RowDetailsVisibility {
  const [visible, setVisible] = useState(true);

  return {
    visible,
    onToggle: () => setVisible((open) => !open),
    onClose: () => setVisible(false),
  };
}

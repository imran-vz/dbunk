import { useState } from "react";

import type { QueryOutcome } from "@/lib/store";

export interface QueryOutcomeState {
  lastOutcome: QueryOutcome | null;
  errorMessage: string | null;
  setOutcome: (outcome: QueryOutcome | null) => void;
}

// Render-phase reset on tab switch — the panel instance is reused
// across tabs (no React key), so without this an error banner from
// tab A could leak into tab B's view. Using the tracked-prev-prop
// pattern (React docs: "Resetting all state when a prop changes")
// rather than a useEffect so biome's exhaustive-deps rule stays
// happy with the trigger-only dependency.
export function useQueryOutcome(tabId: string): QueryOutcomeState {
  const [trackedTabId, setTrackedTabId] = useState(tabId);
  const [lastOutcome, setLastOutcome] = useState<QueryOutcome | null>(null);

  if (trackedTabId !== tabId) {
    setTrackedTabId(tabId);
    setLastOutcome(null);
  }

  return {
    lastOutcome,
    errorMessage: lastOutcome?.kind === "failed" ? lastOutcome.reason : null,
    setOutcome: setLastOutcome,
  };
}

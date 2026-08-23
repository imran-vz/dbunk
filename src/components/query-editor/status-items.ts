import { connectionStatusItem } from "@/components/connection-status";
import type { StatusBarItem } from "@/components/status-bar";
import type { Connection, QuerySessionState } from "@/lib/store";

import type { MonacoPosition } from "./monaco-types";

interface BuildQueryStatusItemsArgs {
  tabLabel: string;
  cursor: MonacoPosition;
  errorMessage: string | null;
  activeConnection: Connection | undefined;
  session?: QuerySessionState;
  /** Staged mutation count — renders the pending badge (§3.1). */
  stagedChangeCount?: number;
  /** Opens the mutation review panel; wired to the badge click. */
  onOpenReview?: () => void;
}

export function buildQueryStatusItems({
  tabLabel,
  cursor,
  errorMessage,
  activeConnection,
  session,
  stagedChangeCount = 0,
  onOpenReview,
}: BuildQueryStatusItemsArgs): StatusBarItem[] {
  return [
    ...(stagedChangeCount > 0
      ? [
          {
            id: "pending",
            value: `${stagedChangeCount} staged`,
            tone: "warning",
            onClick: onOpenReview,
          } satisfies StatusBarItem,
        ]
      : []),
    connectionStatusItem(activeConnection),
    {
      id: "query-session",
      label: "Session",
      value: session
        ? `${session.state} · ${session.transaction.mode} · ${session.transaction.status}`
        : "Legacy",
    },
    {
      id: "tab",
      label: "Tab",
      value: tabLabel,
    },
    {
      id: "cursor",
      label: "",
      value: `Ln ${cursor.lineNumber}, Col ${cursor.column}`,
    },
    {
      id: "diagnostics",
      tone: errorMessage ? "danger" : "healthy",
      value: errorMessage ? "Has errors" : "No errors",
      align: "right",
    },
  ];
}

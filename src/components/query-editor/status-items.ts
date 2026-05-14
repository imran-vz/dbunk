import { connectionStatusItem } from "@/components/connection-status";
import type { StatusBarItem } from "@/components/status-bar";
import type { Connection } from "@/lib/store";

import type { MonacoPosition } from "./monaco-types";

interface BuildQueryStatusItemsArgs {
  tabLabel: string;
  cursor: MonacoPosition;
  errorMessage: string | null;
  activeConnection: Connection | undefined;
}

export function buildQueryStatusItems({
  tabLabel,
  cursor,
  errorMessage,
  activeConnection,
}: BuildQueryStatusItemsArgs): StatusBarItem[] {
  return [
    connectionStatusItem(activeConnection),
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

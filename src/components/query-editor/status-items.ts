import type { StatusBarItem } from "@/components/status-bar";

import type { MonacoPosition } from "./monaco-types";

interface BuildQueryStatusItemsArgs {
  tabLabel: string;
  cursor: MonacoPosition;
  errorMessage: string | null;
}

export function buildQueryStatusItems({
  tabLabel,
  cursor,
  errorMessage,
}: BuildQueryStatusItemsArgs): StatusBarItem[] {
  return [
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
    },
    {
      id: "tx",
      label: "Auto-commit",
      tone: "healthy",
      value: "ON",
      align: "right",
    },
  ];
}

import type { StatusBarItem } from "@/components/status-bar";
import type { StatusTone } from "@/components/ui/status-dot";
import type { Connection } from "@/lib/store";

export function connectionStatusTone(status: Connection["status"]): StatusTone {
  if (status === "Connected") return "healthy";
  if (status === "Read only") return "warning";
  return "neutral";
}

export function connectionStatusItem(
  connection: Connection | undefined,
): StatusBarItem {
  if (!connection) {
    return {
      id: "connection",
      label: "Connection",
      tone: "neutral",
      value: "No connection",
    };
  }
  return {
    id: "connection",
    label: "Connection",
    tone: connectionStatusTone(connection.status),
    value: `${connection.name} · ${connection.status}`,
  };
}

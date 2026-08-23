import type { StatusBarItem } from "@/components/status-bar";
import type { StatusTone } from "@/components/ui/status-dot";
import { ENVIRONMENT_META, resolveSafetyPolicy } from "@/lib/safety-policy";
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
  const environment = resolveSafetyPolicy(connection).environment;
  return {
    id: "connection",
    label: "Connection",
    tone:
      environment === "production"
        ? "danger"
        : connectionStatusTone(connection.status),
    value: `${connection.name} · ${ENVIRONMENT_META[environment].label} · ${connection.status}`,
  };
}

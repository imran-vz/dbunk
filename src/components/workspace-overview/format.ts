import type { DatabaseOverviewStatsStatus } from "@/lib/store";

// oxlint-disable-next-line anti-slop/no-unknown-parameters -- The value is handled at a typed library or domain boundary here.
export function formatConnectionLatency(latency: unknown): string | null {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof latency !== "string") {
    return null;
  }
  const normalized = latency.trim().toLowerCase();
  if (
    normalized === "" ||
    normalized === "--" ||
    normalized === "undefined ms" ||
    normalized === "null ms" ||
    normalized === "nan ms"
  ) {
    return null;
  }
  return latency;
}

export function formatLastChecked(value: string | undefined): string {
  if (!value || value === "Never" || value === "Just now") {
    return value === "Never" ? "never" : "just now";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const diffMs = Date.now() - date.getTime();
  if (diffMs < 60_000) return "just now";
  if (diffMs < 3_600_000) {
    const minutes = Math.round(diffMs / 60_000);
    return `${minutes} min ago`;
  }
  if (diffMs < 86_400_000) {
    const hours = Math.round(diffMs / 3_600_000);
    return `${hours} hr ago`;
  }
  return date.toLocaleString();
}

export function formatByteStat(
  value: number | undefined,
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof value === "number") {
    return formatBytes(value);
  }
  if (status?.state === "loading") {
    return "…";
  }
  if (status?.state === "error") {
    return "Unavailable";
  }
  return "—";
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) {
    return "—";
  }
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const precision =
    Number.isInteger(value) || value >= 10 || unitIndex === 0 ? 0 : 1;
  return `${value.toFixed(precision)} ${units[unitIndex]}`;
}

export function pendingMetric(
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  if (status?.state === "loading") return "…";
  if (status?.state === "error") return "Unavailable";
  return "—";
}

// PG returns reltuples as a planner estimate — use compact suffixes for the
// dashboard (1.2M, 18.2K). Precise counts would need per-table SELECT count(*)
// which can be expensive on large databases.
export function formatRowCount(
  value: number | undefined,
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  // oxlint-disable-next-line anti-slop/no-runtime-typeof -- The value is handled at a typed library or domain boundary here.
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return pendingMetric(status);
  }
  if (value < 1000) return value.toLocaleString();
  const units: Array<[number, string]> = [
    [1_000_000_000, "B"],
    [1_000_000, "M"],
    [1_000, "K"],
  ];
  for (const [scale, suffix] of units) {
    if (value >= scale) {
      const scaled = value / scale;
      const precision = scaled >= 10 ? 0 : 1;
      return `${scaled.toFixed(precision)}${suffix}`;
    }
  }
  return value.toLocaleString();
}

export function metricCount(
  value: number | undefined,
  status: DatabaseOverviewStatsStatus | undefined,
): string {
  return value !== undefined ? value.toLocaleString() : pendingMetric(status);
}

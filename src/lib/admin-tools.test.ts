import { describe, expect, it } from "vitest";
import {
  buildBlockerChains,
  cacheHitRatioTone,
  DB_STATS_SQL,
  LOCK_MANAGER_SQL,
  maintenanceActionSql,
  PENDING_TRANSACTIONS_SQL,
  SESSION_MANAGER_SQL,
  sessionActionSql,
} from "@/lib/admin-tools";

describe("Postgres admin tools", () => {
  it("exposes manager queries for sessions, locks, transactions, and stats", () => {
    expect(SESSION_MANAGER_SQL).toContain("pg_stat_activity");
    expect(LOCK_MANAGER_SQL).toContain("pg_locks");
    expect(PENDING_TRANSACTIONS_SQL).toContain("idle in transaction");
    expect(DB_STATS_SQL).toContain("cache_hit_ratio");
  });

  it("builds cancel and terminate actions", () => {
    expect(sessionActionSql("cancel", 123)).toBe(
      "SELECT pg_cancel_backend(123);",
    );
    expect(sessionActionSql("terminate", 123)).toBe(
      "SELECT pg_terminate_backend(123);",
    );
  });

  it("builds table maintenance actions", () => {
    expect(maintenanceActionSql("vacuum", "public", "users")).toBe(
      'VACUUM "public"."users";',
    );
    expect(maintenanceActionSql("reindex", "public", "users")).toBe(
      'REINDEX TABLE "public"."users";',
    );
  });

  it("builds blocker chains from lock rows", () => {
    expect(
      buildBlockerChains([
        { pid: 10, blockedBy: [20] },
        { pid: 20, blockedBy: [30] },
        { pid: 30, blockedBy: [] },
      ]),
    ).toEqual([
      [10, 20, 30],
      [20, 30],
    ]);
  });

  it("classifies cache hit ratio health", () => {
    expect(cacheHitRatioTone(0.995)).toBe("healthy");
    expect(cacheHitRatioTone(0.97)).toBe("warning");
    expect(cacheHitRatioTone(0.9)).toBe("danger");
  });
});

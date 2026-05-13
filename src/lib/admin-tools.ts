export type SessionAction = "cancel" | "terminate";
export type MaintenanceAction = "vacuum" | "analyze" | "reindex";

export type LockRow = {
  pid: number;
  blockedBy: number[];
};

const quoteIdentifier = (identifier: string): string =>
  `"${identifier.replace(/"/g, '""')}"`;

const qualified = (schema: string, table: string): string =>
  `${quoteIdentifier(schema)}.${quoteIdentifier(table)}`;

export const SESSION_MANAGER_SQL = `
SELECT pid, usename, application_name, client_addr, state, wait_event_type,
       wait_event, query_start, xact_start, query
FROM pg_stat_activity
ORDER BY query_start NULLS LAST, pid
`.trim();

export const LOCK_MANAGER_SQL = `
SELECT blocked.pid AS blocked_pid,
       blocking.pid AS blocking_pid,
       blocked.query AS blocked_query,
       blocking.query AS blocking_query
FROM pg_locks blocked_lock
JOIN pg_stat_activity blocked ON blocked.pid = blocked_lock.pid
JOIN pg_locks blocking_lock
  ON blocking_lock.locktype = blocked_lock.locktype
 AND blocking_lock.database IS NOT DISTINCT FROM blocked_lock.database
 AND blocking_lock.relation IS NOT DISTINCT FROM blocked_lock.relation
 AND blocking_lock.page IS NOT DISTINCT FROM blocked_lock.page
 AND blocking_lock.tuple IS NOT DISTINCT FROM blocked_lock.tuple
 AND blocking_lock.virtualxid IS NOT DISTINCT FROM blocked_lock.virtualxid
 AND blocking_lock.transactionid IS NOT DISTINCT FROM blocked_lock.transactionid
 AND blocking_lock.classid IS NOT DISTINCT FROM blocked_lock.classid
 AND blocking_lock.objid IS NOT DISTINCT FROM blocked_lock.objid
 AND blocking_lock.objsubid IS NOT DISTINCT FROM blocked_lock.objsubid
JOIN pg_stat_activity blocking ON blocking.pid = blocking_lock.pid
WHERE NOT blocked_lock.granted AND blocking_lock.granted
`.trim();

export const PENDING_TRANSACTIONS_SQL = `
SELECT pid, usename, state, xact_start, now() - xact_start AS age, query
FROM pg_stat_activity
WHERE xact_start IS NOT NULL
  AND state IN ('idle in transaction', 'active')
ORDER BY xact_start ASC
`.trim();

export const DB_STATS_SQL = `
SELECT datname,
       pg_database_size(datname) AS database_size_bytes,
       blks_hit,
       blks_read,
       CASE WHEN blks_hit + blks_read = 0 THEN 1
            ELSE blks_hit::numeric / (blks_hit + blks_read)
       END AS cache_hit_ratio,
       xact_commit,
       xact_rollback
FROM pg_stat_database
WHERE datname = current_database()
`.trim();

export function sessionActionSql(action: SessionAction, pid: number): string {
  const fn = action === "cancel" ? "pg_cancel_backend" : "pg_terminate_backend";
  return `SELECT ${fn}(${pid});`;
}

export function maintenanceActionSql(
  action: MaintenanceAction,
  schema: string,
  table: string,
): string {
  const relation = qualified(schema, table);
  switch (action) {
    case "vacuum":
      return `VACUUM ${relation};`;
    case "analyze":
      return `ANALYZE ${relation};`;
    case "reindex":
      return `REINDEX TABLE ${relation};`;
  }
}

export function buildBlockerChains(rows: LockRow[]): number[][] {
  const byPid = new Map(rows.map((row) => [row.pid, row.blockedBy]));
  return rows
    .filter((row) => row.blockedBy.length > 0)
    .map((row) => {
      const chain = [row.pid];
      const seen = new Set(chain);
      let blockers = row.blockedBy;
      while (blockers.length > 0) {
        const blocker = blockers[0];
        if (seen.has(blocker)) {
          break;
        }
        chain.push(blocker);
        seen.add(blocker);
        blockers = byPid.get(blocker) ?? [];
      }
      return chain;
    });
}

export function cacheHitRatioTone(
  ratio: number,
): "healthy" | "warning" | "danger" {
  if (ratio >= 0.99) {
    return "healthy";
  }
  if (ratio >= 0.95) {
    return "warning";
  }
  return "danger";
}

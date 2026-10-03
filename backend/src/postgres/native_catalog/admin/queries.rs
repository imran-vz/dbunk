// Every variable-width wire field is guarded before transfer. The extra row
// proves section truncation; complete blocker arrays never cross the wire.
pub(super) const SESSIONS: &str = r#"
WITH selected AS MATERIALIZED (
 SELECT a.*, a.client_addr::text AS address_text, pg_catalog.left(a.query,500) AS clipped_query
 FROM pg_catalog.pg_stat_activity a
 WHERE (a.datname=pg_catalog.current_database() OR a.datname IS NULL)
   AND (NOT $1::boolean OR EXTRACT(EPOCH FROM pg_catalog.transaction_timestamp()-a.xact_start)::bigint > 0)
 ORDER BY CASE WHEN $1::boolean THEN a.xact_start END NULLS LAST,
          a.state NULLS LAST,a.query_start NULLS LAST,a.pid
 LIMIT $2
)
SELECT pid,
 CASE WHEN pg_catalog.octet_length(usename::text)<=2048 THEN usename::text END AS usename,
 CASE WHEN pg_catalog.octet_length(datname::text)<=2048 THEN datname::text END AS datname,
 CASE WHEN pg_catalog.octet_length(application_name)<=2048 THEN application_name END AS application_name,
 CASE WHEN pg_catalog.octet_length(address_text)<=2048 THEN address_text END AS client_addr,
 CASE WHEN pg_catalog.octet_length(state)<=2048 THEN state END AS state,
 CASE WHEN pg_catalog.octet_length(wait_event_type)<=2048 THEN wait_event_type END AS wait_event_type,
 CASE WHEN pg_catalog.octet_length(wait_event)<=2048 THEN wait_event END AS wait_event,
 clipped_query AS query,
 COALESCE(pg_catalog.length(query)>500,false) AS query_clipped,
 COALESCE(query='<insufficient privilege>',false) AS details_restricted,
 EXTRACT(EPOCH FROM pg_catalog.transaction_timestamp()-query_start)::bigint AS query_age_seconds,
 EXTRACT(EPOCH FROM pg_catalog.transaction_timestamp()-xact_start)::bigint AS transaction_age_seconds,
 pg_catalog.to_char(backend_start AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS backend_start,
 pg_catalog.to_char(query_start AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS query_start,
 pg_catalog.to_char(xact_start AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS xact_start,
 COALESCE(GREATEST(pg_catalog.octet_length(usename::text),pg_catalog.octet_length(datname::text),
   pg_catalog.octet_length(application_name),pg_catalog.octet_length(address_text),pg_catalog.octet_length(state),
   pg_catalog.octet_length(wait_event_type),pg_catalog.octet_length(wait_event))>2048,false) AS too_large
FROM selected
"#;

pub(super) const LOCKS: &str = r#"
WITH selected AS MATERIALIZED (
 SELECT l.pid,l.locktype,l.mode,l.granted,a.query,a.backend_start,a.query_start,
   CASE WHEN l.database=(SELECT oid FROM pg_catalog.pg_database WHERE datname=pg_catalog.current_database())
        THEN l.relation::pg_catalog.regclass::text END AS relation_name
 FROM pg_catalog.pg_locks l LEFT JOIN pg_catalog.pg_stat_activity a ON a.pid=l.pid
 WHERE a.datname=pg_catalog.current_database() OR a.datname IS NULL
 ORDER BY l.granted ASC,l.pid NULLS LAST,l.locktype,l.mode,l.relation,l.virtualtransaction
 LIMIT $1
), blocked AS MATERIALIZED (
 SELECT selected.*,pg_catalog.pg_blocking_pids(pid) AS blocker_ids FROM selected
)
SELECT pid,
 CASE WHEN pg_catalog.octet_length(locktype)<=2048 THEN locktype END AS locktype,
 CASE WHEN pg_catalog.octet_length(relation_name)<=2048 THEN relation_name END AS relation,
 CASE WHEN pg_catalog.octet_length(mode)<=2048 THEN mode END AS mode,
 granted,blocker_ids[1:64] AS blocked_by,
 COALESCE(pg_catalog.cardinality(blocker_ids)>64,false) AS blocked_by_clipped,
 pg_catalog.left(query,500) AS query,
 COALESCE(pg_catalog.length(query)>500,false) AS query_clipped,
 COALESCE(query='<insufficient privilege>',false) AS details_restricted,
 pg_catalog.to_char(backend_start AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS backend_start,
 pg_catalog.to_char(query_start AT TIME ZONE 'UTC','YYYY-MM-DD"T"HH24:MI:SS.US"Z"') AS query_start,
 COALESCE(GREATEST(pg_catalog.octet_length(locktype),pg_catalog.octet_length(relation_name),pg_catalog.octet_length(mode))>2048,false) AS too_large
FROM blocked
"#;

pub(super) const STATS: &str = r#"
SELECT pg_catalog.pg_database_size(datid)::bigint AS database_size_bytes,
 CASE WHEN blks_hit::numeric+blks_read::numeric=0 THEN NULL
      ELSE (blks_hit::numeric/(blks_hit::numeric+blks_read::numeric))::float8 END AS cache_hit_ratio,
 (SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity WHERE datname=pg_catalog.current_database() AND state='active') AS active_sessions,
 (SELECT count(*)::bigint FROM pg_catalog.pg_stat_activity WHERE datname=pg_catalog.current_database() AND state='idle in transaction') AS idle_in_transaction,
 (SELECT count(*)::bigint FROM pg_catalog.pg_locks WHERE NOT granted) AS blocked_locks,
 EXISTS(SELECT 1 FROM pg_catalog.pg_stat_activity WHERE datname=pg_catalog.current_database() AND query='<insufficient privilege>') AS activity_restricted
FROM pg_catalog.pg_stat_database WHERE datname=pg_catalog.current_database()
"#;

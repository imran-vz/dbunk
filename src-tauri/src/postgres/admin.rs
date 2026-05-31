//! Server details, overview stats, relation stats, admin snapshots, and maintenance.

use std::time::Instant;

use sqlx::{Executor, Row};

use crate::{
    quote_double, DatabaseOverviewStats, ExecuteDdlResult, PgAdminSnapshot, PgAdminStats,
    PgBackendActionResult, PgLockInfo, PgPendingTransactionInfo, PgSessionInfo, StoredConnection,
};

use super::connect;

pub async fn load_database_overview_stats(
    connection: &StoredConnection,
) -> Result<DatabaseOverviewStats, String> {
    let mut conn = connect(connection).await?;

    let row = sqlx::query(
        r#"
        WITH user_relations AS (
            SELECT c.oid, c.relkind, c.reltuples, n.nspname
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND n.nspname NOT LIKE 'pg_toast%'
        )
        SELECT
            pg_database_size(current_database())::bigint AS database_size_bytes,
            COALESCE(SUM(pg_table_size(oid)) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS table_size_bytes,
            COALESCE(SUM(pg_indexes_size(oid)) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS index_size_bytes,
            COUNT(*) FILTER (WHERE relkind IN ('r', 'p'))::bigint AS table_count,
            COUNT(DISTINCT nspname)::bigint AS schema_count,
            COALESCE(SUM(GREATEST(reltuples, 0)::bigint) FILTER (WHERE relkind IN ('r', 'p')), 0)::bigint AS row_count_estimate,
            COUNT(*) FILTER (WHERE relkind = 'i')::bigint AS index_count,
            (SELECT COUNT(*) FROM pg_stat_activity WHERE datname = current_database())::bigint AS connection_count
        FROM user_relations
        "#,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    Ok(DatabaseOverviewStats {
        database_size_bytes: row.try_get("database_size_bytes").unwrap_or(0),
        table_size_bytes: row.try_get("table_size_bytes").unwrap_or(0),
        index_size_bytes: row.try_get("index_size_bytes").unwrap_or(0),
        table_count: row.try_get("table_count").unwrap_or(0),
        schema_count: row.try_get("schema_count").unwrap_or(0),
        row_count_estimate: row.try_get("row_count_estimate").unwrap_or(0),
        index_count: row.try_get("index_count").unwrap_or(0),
        connection_count: row.try_get("connection_count").unwrap_or(0),
    })
}

pub async fn load_relation_stats(
    connection: &StoredConnection,
) -> Result<Vec<crate::RelationInfo>, String> {
    let mut conn = connect(connection).await?;

    let rows = sqlx::query(
        r#"
        SELECT
            n.nspname AS schema,
            c.relname AS name,
            CASE c.relkind
                WHEN 'r' THEN 'table'
                WHEN 'p' THEN 'table'
                WHEN 'v' THEN 'view'
                WHEN 'm' THEN 'materialized view'
                ELSE c.relkind::text
            END AS kind,
            CASE
                WHEN c.relkind IN ('r', 'p', 'm')
                    THEN GREATEST(c.reltuples, 0)::bigint
                ELSE 0::bigint
            END AS row_count_estimate,
            CASE
                WHEN c.relkind IN ('r', 'p', 'm')
                    THEN pg_total_relation_size(c.oid)::bigint
                ELSE 0::bigint
            END AS total_size_bytes
        FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind IN ('r', 'p', 'v', 'm')
          AND c.relispartition IS NOT TRUE
          AND n.nspname NOT IN ('pg_catalog', 'information_schema')
          AND n.nspname NOT LIKE 'pg_toast%'
        ORDER BY n.nspname, c.relname
        "#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    Ok(rows
        .into_iter()
        .map(|row| crate::RelationInfo {
            schema: row.try_get("schema").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            kind: row.try_get("kind").unwrap_or_default(),
            row_count_estimate: row.try_get("row_count_estimate").unwrap_or(0),
            total_size_bytes: row.try_get("total_size_bytes").unwrap_or(0),
        })
        .collect())
}

pub async fn load_server_details(
    connection: &StoredConnection,
) -> Result<crate::ServerDetails, String> {
    let mut conn = connect(connection).await?;

    let summary = sqlx::query(
        r#"
        SELECT
            version() AS server_version,
            current_setting('server_encoding') AS encoding,
            current_setting('lc_collate') AS locale,
            current_setting('timezone') AS timezone
        "#,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let settings_rows = sqlx::query(
        r#"
        SELECT name, setting, unit, category, short_desc, source, boot_val, reset_val
        FROM pg_settings
        ORDER BY category, name
        "#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let extension_rows = sqlx::query(
        r#"
        SELECT
            e.extname AS name,
            e.extversion AS version,
            n.nspname AS schema,
            pg_catalog.obj_description(e.oid, 'pg_extension') AS description
        FROM pg_extension e
        JOIN pg_namespace n ON n.oid = e.extnamespace
        ORDER BY e.extname
        "#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;

    let settings = settings_rows
        .into_iter()
        .map(|row| crate::PgSetting {
            name: row.try_get("name").unwrap_or_default(),
            setting: row.try_get("setting").unwrap_or_default(),
            unit: row.try_get::<Option<String>, _>("unit").unwrap_or(None),
            category: row.try_get("category").unwrap_or_default(),
            short_desc: row
                .try_get::<Option<String>, _>("short_desc")
                .unwrap_or(None),
            source: row.try_get("source").unwrap_or_default(),
            boot_val: row.try_get::<Option<String>, _>("boot_val").unwrap_or(None),
            reset_val: row
                .try_get::<Option<String>, _>("reset_val")
                .unwrap_or(None),
        })
        .collect();

    let extensions = extension_rows
        .into_iter()
        .map(|row| crate::PgExtension {
            name: row.try_get("name").unwrap_or_default(),
            version: row.try_get("version").unwrap_or_default(),
            schema: row.try_get("schema").unwrap_or_default(),
            description: row
                .try_get::<Option<String>, _>("description")
                .unwrap_or(None),
        })
        .collect();

    Ok(crate::ServerDetails {
        server_version: summary.try_get("server_version").unwrap_or_default(),
        encoding: summary.try_get("encoding").unwrap_or_default(),
        locale: summary.try_get("locale").unwrap_or_default(),
        timezone: summary.try_get("timezone").unwrap_or_default(),
        settings,
        extensions,
    })
}

pub async fn load_admin_snapshot(connection: &StoredConnection) -> Result<PgAdminSnapshot, String> {
    let mut conn = connect(connection).await?;
    let session_rows = sqlx::query(
        r#"
        SELECT pid, usename, datname, application_name, client_addr::text, state,
               wait_event_type, wait_event,
               EXTRACT(EPOCH FROM now() - query_start)::bigint AS query_age_seconds,
               EXTRACT(EPOCH FROM now() - xact_start)::bigint AS transaction_age_seconds,
               left(query, 500) AS query
        FROM pg_stat_activity
        WHERE datname = current_database() OR datname IS NULL
        ORDER BY state NULLS LAST, query_start NULLS LAST
        "#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    let sessions = session_rows
        .into_iter()
        .map(|row| PgSessionInfo {
            pid: row.try_get("pid").unwrap_or_default(),
            user: row.try_get("usename").unwrap_or_default(),
            database: row.try_get("datname").ok(),
            application_name: row.try_get("application_name").unwrap_or_default(),
            client_addr: row.try_get("client_addr").ok(),
            state: row.try_get("state").ok(),
            wait_event_type: row.try_get("wait_event_type").ok(),
            wait_event: row.try_get("wait_event").ok(),
            query_age_seconds: row.try_get("query_age_seconds").ok(),
            transaction_age_seconds: row.try_get("transaction_age_seconds").ok(),
            query: row.try_get("query").unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let lock_rows = sqlx::query(
        r#"
        SELECT l.pid, l.locktype, l.relation::regclass::text AS relation,
               l.mode, l.granted, pg_blocking_pids(l.pid) AS blocked_by,
               left(a.query, 500) AS query
        FROM pg_locks l
        LEFT JOIN pg_stat_activity a ON a.pid = l.pid
        WHERE a.datname = current_database() OR a.datname IS NULL
        ORDER BY l.granted ASC, l.pid, l.locktype
        "#,
    )
    .fetch_all(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    let locks = lock_rows
        .into_iter()
        .map(|row| PgLockInfo {
            pid: row.try_get("pid").unwrap_or_default(),
            lock_type: row.try_get("locktype").unwrap_or_default(),
            relation: row.try_get("relation").ok(),
            mode: row.try_get("mode").unwrap_or_default(),
            granted: row.try_get("granted").unwrap_or(false),
            blocked_by: row.try_get("blocked_by").unwrap_or_default(),
            query: row.try_get("query").unwrap_or_default(),
        })
        .collect::<Vec<_>>();

    let pending_transactions = sessions
        .iter()
        .filter(|session| session.transaction_age_seconds.unwrap_or(0) > 0)
        .map(|session| PgPendingTransactionInfo {
            pid: session.pid,
            user: session.user.clone(),
            state: session.state.clone(),
            transaction_age_seconds: session.transaction_age_seconds,
            query: session.query.clone(),
        })
        .collect::<Vec<_>>();

    let stats_row = sqlx::query(
        r#"
        SELECT pg_database_size(current_database())::bigint AS database_size_bytes,
               CASE WHEN blks_hit + blks_read = 0 THEN NULL
                    ELSE blks_hit::float8 / (blks_hit + blks_read)::float8
               END AS cache_hit_ratio,
               (SELECT count(*)::bigint FROM pg_stat_activity WHERE datname = current_database() AND state = 'active') AS active_sessions,
               (SELECT count(*)::bigint FROM pg_stat_activity WHERE datname = current_database() AND state = 'idle in transaction') AS idle_in_transaction,
               (SELECT count(*)::bigint FROM pg_locks WHERE NOT granted) AS blocked_locks
        FROM pg_stat_database
        WHERE datname = current_database()
        "#,
    )
    .fetch_one(&mut *conn)
    .await
    .map_err(|error| error.to_string())?;
    let stats = PgAdminStats {
        database_size_bytes: stats_row.try_get("database_size_bytes").unwrap_or(0),
        cache_hit_ratio: stats_row.try_get("cache_hit_ratio").ok(),
        active_sessions: stats_row.try_get("active_sessions").unwrap_or(0),
        idle_in_transaction: stats_row.try_get("idle_in_transaction").unwrap_or(0),
        blocked_locks: stats_row.try_get("blocked_locks").unwrap_or(0),
    };
    Ok(PgAdminSnapshot {
        sessions,
        locks,
        pending_transactions,
        stats,
    })
}

pub async fn cancel_backend(
    connection: &StoredConnection,
    pid: i32,
) -> Result<PgBackendActionResult, String> {
    let mut conn = connect(connection).await?;
    let ok = sqlx::query_scalar::<_, bool>("SELECT pg_cancel_backend($1)")
        .bind(pid)
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(PgBackendActionResult { ok })
}

pub async fn terminate_backend(
    connection: &StoredConnection,
    pid: i32,
) -> Result<PgBackendActionResult, String> {
    let mut conn = connect(connection).await?;
    let ok = sqlx::query_scalar::<_, bool>("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(PgBackendActionResult { ok })
}

pub async fn run_maintenance(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    action: &str,
) -> Result<ExecuteDdlResult, String> {
    let mut conn = connect(connection).await?;
    let start = Instant::now();
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let sql = match action {
        "vacuum" => format!("VACUUM {qualified}"),
        "analyze" => format!("ANALYZE {qualified}"),
        "reindex" => format!("REINDEX TABLE {qualified}"),
        _ => return Err("maintenance action must be vacuum, analyze, or reindex".to_string()),
    };
    conn.execute(sql.as_str())
        .await
        .map_err(|error| error.to_string())?;
    Ok(ExecuteDdlResult {
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

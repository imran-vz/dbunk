//! Table Seeding for the sqlx-Any engines (MySQL, SQLite) — ADR-0020.
//!
//! Same shape as `postgres::seed`: build a plan from the introspected
//! structure, sample foreign-key parents and unique-integer maxima from
//! the live table, then bulk-insert generated rows inside one
//! transaction so a rejected batch rolls the whole run back.
//!
//! Only the SQL differs from the PostgreSQL path — identifier quoting,
//! `?` placeholders instead of `$n`, and each engine's own bind-count
//! ceiling. Everything about *what* to generate lives in `crate::seed`.

use std::time::Instant;

use sqlx::{Any, AnyConnection, Connection};

use crate::seed::{
    analyze_plan, finalize_plan, generate_rows_from, insert_columns, SeedDialect, SeedRng,
};
use crate::{
    qualified_table_name, quote_backtick, quote_double, DatabaseEngine, ForeignKeyInfo,
    SeedColumnSpec, SeedTableResult, StoredConnection,
};

use super::relational::{row_to_strings, sqlx_connect};

/// MySQL's protocol caps placeholders at 65535 per statement.
const MYSQL_MAX_PARAMS: usize = 60_000;
/// SQLite's `SQLITE_MAX_VARIABLE_NUMBER` is 999 on builds older than
/// 3.32 — stay under the oldest ceiling rather than probe for it.
const SQLITE_MAX_PARAMS: usize = 900;
/// How many distinct parent rows to sample per foreign key.
const FK_POOL_LIMIT: usize = 1_000;

fn dialect_of(engine: &DatabaseEngine) -> Result<SeedDialect, String> {
    match engine {
        DatabaseEngine::MySQL => Ok(SeedDialect::MySql),
        DatabaseEngine::SQLite => Ok(SeedDialect::Sqlite),
        other => Err(format!(
            "BUG: sqlx-Any seeding reached for {}",
            super::relational::engine_name(other)
        )),
    }
}

fn quote(engine: &DatabaseEngine, identifier: &str) -> String {
    match engine {
        DatabaseEngine::MySQL => quote_backtick(identifier),
        _ => quote_double(identifier),
    }
}

fn max_params(engine: &DatabaseEngine) -> usize {
    match engine {
        DatabaseEngine::MySQL => MYSQL_MAX_PARAMS,
        _ => SQLITE_MAX_PARAMS,
    }
}

pub(super) async fn seed_table<F>(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    row_count: u32,
    seed: u64,
    specs: &[SeedColumnSpec],
    report_progress: F,
) -> Result<SeedTableResult, String>
where
    F: Fn(u64) + Send + Sync,
{
    if row_count == 0 {
        return Err("row count must be at least 1".to_string());
    }
    let engine = connection.engine();
    let dialect = dialect_of(&engine)?;

    let structure = super::relational::fetch_table_structure(connection, schema, table).await?;
    // The columns-only fallback reports no constraints and no types, so
    // a plan built from it could not honour NOT NULL, FK or unique —
    // refuse rather than generate rows the table will reject.
    if !structure.capabilities.foreign_keys {
        return Err(format!(
            "cannot seed \"{table}\": its constraints could not be read from the catalog, \
             so generated rows could not be guaranteed valid"
        ));
    }
    let start = Instant::now();

    let draft = analyze_plan(dialect, &structure, specs)?;

    let mut conn = sqlx_connect(connection).await?;
    let mut fk_pools: Vec<Option<Vec<Vec<String>>>> = vec![None; structure.foreign_keys.len()];
    for fk_index in &draft.needed_pools {
        let pool = sample_fk_pool(
            &mut conn,
            &engine,
            schema,
            &structure.foreign_keys[*fk_index],
        )
        .await?;
        fk_pools[*fk_index] = Some(pool);
    }
    let mut maxes: Vec<(String, i64)> = Vec::with_capacity(draft.needed_maxes.len());
    for column in &draft.needed_maxes {
        maxes.push((
            column.clone(),
            fetch_integer_max(&mut conn, &engine, schema, table, column).await?,
        ));
    }

    let plan = finalize_plan(
        &structure,
        draft,
        fk_pools,
        &maxes,
        chrono::Utc::now().timestamp(),
    )?;

    let columns = insert_columns(&plan);
    if columns.is_empty() {
        return Err("nothing to seed: every column is skipped".to_string());
    }

    let mut rng = SeedRng::new(seed);
    let chunk_size = (max_params(&engine) / columns.len()).clamp(1, 500);
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let mut rows_inserted: u64 = 0;
    let mut rows_generated: u32 = 0;
    while rows_generated < row_count {
        let batch_size = (row_count - rows_generated).min(chunk_size as u32);
        let rows = generate_rows_from(&plan, rows_generated, batch_size, &mut rng);
        let (sql, params) = build_bulk_insert(&engine, schema, table, &columns, &rows);
        let mut query = sqlx::query::<Any>(&sql);
        for param in &params {
            query = query.bind(param.as_deref());
        }
        rows_inserted += query
            .execute(&mut *tx)
            .await
            .map_err(|error| error.to_string())?
            .rows_affected();
        rows_generated += batch_size;
        report_progress(u64::from(rows_generated));
    }
    tx.commit().await.map_err(|error| error.to_string())?;

    Ok(SeedTableResult {
        rows_inserted,
        seed_used: seed,
        runtime_ms: start.elapsed().as_millis() as u64,
    })
}

/// Multi-row INSERT with positional placeholders. Values travel as text
/// and are coerced by the engine — both MySQL and SQLite widen string
/// literals into the column's type, so no per-column cast is needed the
/// way PostgreSQL requires one.
fn build_bulk_insert(
    engine: &DatabaseEngine,
    schema: &str,
    table: &str,
    columns: &[String],
    rows: &[Vec<Option<String>>],
) -> (String, Vec<Option<String>>) {
    let qualified = qualified_table_name(engine, schema, table);
    let column_list = columns
        .iter()
        .map(|column| quote(engine, column))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = format!("({})", vec!["?"; columns.len()].join(", "));
    let mut params = Vec::with_capacity(columns.len() * rows.len());
    for row in rows {
        for index in 0..columns.len() {
            params.push(row.get(index).cloned().unwrap_or(None));
        }
    }
    let values = vec![placeholders; rows.len()].join(", ");
    (
        format!("INSERT INTO {qualified} ({column_list}) VALUES {values}"),
        params,
    )
}

/// Sample up to [`FK_POOL_LIMIT`] distinct parent tuples for one FK.
async fn sample_fk_pool(
    conn: &mut AnyConnection,
    engine: &DatabaseEngine,
    child_schema: &str,
    fk: &ForeignKeyInfo,
) -> Result<Vec<Vec<String>>, String> {
    let columns = fk
        .referenced_columns
        .iter()
        .map(|column| quote(engine, column))
        .collect::<Vec<_>>()
        .join(", ");
    let not_null = fk
        .referenced_columns
        .iter()
        .map(|column| format!("{} IS NOT NULL", quote(engine, column)))
        .collect::<Vec<_>>()
        .join(" AND ");
    // SQLite foreign keys never name a schema; MySQL's may point at
    // another database. Falling back to the child's schema keeps the
    // reference resolvable in both.
    let parent_schema = if fk.referenced_schema.is_empty() {
        child_schema
    } else {
        &fk.referenced_schema
    };
    let sql = format!(
        "SELECT DISTINCT {columns} FROM {} WHERE {not_null} LIMIT {FK_POOL_LIMIT}",
        qualified_table_name(engine, parent_schema, &fk.referenced_table)
    );
    let rows = sqlx::query::<Any>(&sql)
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.iter().map(row_to_strings).collect())
}

/// Current MAX of an integer column, so unique sequences start above
/// existing data.
async fn fetch_integer_max(
    conn: &mut AnyConnection,
    engine: &DatabaseEngine,
    schema: &str,
    table: &str,
    column: &str,
) -> Result<i64, String> {
    let sql = format!(
        "SELECT COALESCE(MAX({}), 0) FROM {}",
        quote(engine, column),
        qualified_table_name(engine, schema, table)
    );
    let rows = sqlx::query::<Any>(&sql)
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    // The Any driver decodes MAX() as whatever the column's type maps
    // to, so read it back through the shared stringifier rather than
    // guessing a Rust type.
    Ok(rows
        .first()
        .map(row_to_strings)
        .and_then(|row| row.first().and_then(|value| value.parse::<i64>().ok()))
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bulk_insert_backticks_mysql_identifiers_and_repeats_placeholders() {
        let (sql, params) = build_bulk_insert(
            &DatabaseEngine::MySQL,
            "shop",
            "orders",
            &["id".to_string(), "note".to_string()],
            &[
                vec![Some("1".to_string()), Some("a".to_string())],
                vec![Some("2".to_string()), None],
            ],
        );
        assert_eq!(
            sql,
            "INSERT INTO `shop`.`orders` (`id`, `note`) VALUES (?, ?), (?, ?)"
        );
        assert_eq!(
            params,
            vec![
                Some("1".to_string()),
                Some("a".to_string()),
                Some("2".to_string()),
                None
            ]
        );
    }

    #[test]
    fn bulk_insert_double_quotes_sqlite_identifiers() {
        let (sql, _) = build_bulk_insert(
            &DatabaseEngine::SQLite,
            "main",
            "orders",
            &["id".to_string()],
            &[vec![Some("1".to_string())]],
        );
        assert_eq!(sql, "INSERT INTO \"main\".\"orders\" (\"id\") VALUES (?)");
    }

    #[test]
    fn bulk_insert_pads_short_rows_with_null() {
        let (_, params) = build_bulk_insert(
            &DatabaseEngine::SQLite,
            "main",
            "t",
            &["a".to_string(), "b".to_string()],
            &[vec![Some("1".to_string())]],
        );
        assert_eq!(params, vec![Some("1".to_string()), None]);
    }

    #[test]
    fn sqlite_chunk_stays_under_the_oldest_variable_ceiling() {
        let columns = 3;
        let chunk = (max_params(&DatabaseEngine::SQLite) / columns).clamp(1, 500);
        assert!(chunk * columns <= 999);
    }
}

#[cfg(test)]
mod sqlite_live_tests {
    //! End-to-end seeding against a real SQLite file. SQLite needs no
    //! server, so unlike the other engines' live tests this one runs in
    //! the normal suite and guards the whole path: introspection, plan
    //! building, FK sampling, and the transactional bulk insert.

    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::{SqliteStoredConnection, StoredConnection};

    fn connection(path: &str) -> StoredConnection {
        StoredConnection::SQLite(SqliteStoredConnection {
            id: "seed-sqlite-test".into(),
            name: "seed sqlite test".into(),
            database: path.to_string(),
            host: String::new(),
            port: 0,
            user: String::new(),
            password: String::new(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
        })
    }

    async fn exec(connection: &StoredConnection, sql: &str) {
        let mut conn = sqlx_connect(connection).await.expect("connect");
        sqlx::raw_sql(sql)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    async fn scalar(connection: &StoredConnection, sql: &str) -> String {
        let mut conn = sqlx_connect(connection).await.expect("connect");
        let rows = sqlx::query::<Any>(sql)
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        row_to_strings(&rows[0]).remove(0)
    }

    async fn column_values(connection: &StoredConnection, sql: &str) -> Vec<String> {
        let mut conn = sqlx_connect(connection).await.expect("connect");
        let rows = sqlx::query::<Any>(sql)
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        rows.iter()
            .map(|row| row_to_strings(row).remove(0))
            .collect()
    }

    fn spec(column: &str) -> SeedColumnSpec {
        SeedColumnSpec {
            column: column.to_string(),
            skip: false,
            constant: None,
            values: None,
            generator: None,
            min: None,
            max: None,
            null_rate: None,
        }
    }

    async fn fixture() -> (tempfile::NamedTempFile, StoredConnection) {
        let file = tempfile::NamedTempFile::new().expect("temp db");
        let connection = connection(file.path().to_str().expect("utf-8 path"));
        exec(
            &connection,
            "CREATE TABLE customers (
               id INTEGER PRIMARY KEY,
               email TEXT NOT NULL UNIQUE,
               full_name TEXT NOT NULL,
               city TEXT,
               age INTEGER
             )",
        )
        .await;
        exec(
            &connection,
            "CREATE TABLE orders (
               id INTEGER PRIMARY KEY,
               customer_id INTEGER NOT NULL REFERENCES customers(id),
               total REAL NOT NULL,
               note VARCHAR(24)
             )",
        )
        .await;
        (file, connection)
    }

    #[tokio::test]
    async fn seeds_parent_then_child_without_orphans() {
        let (_file, connection) = fixture().await;

        // Child first must fail fast: the parent has no rows to sample.
        let error = seed_table(&connection, "main", "orders", 10, 42, &[], |_| {})
            .await
            .expect_err("empty parent must fail");
        assert!(error.contains("customers"), "unexpected error: {error}");
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM orders").await,
            "0"
        );

        let progress = AtomicU64::new(0);
        let result = seed_table(&connection, "main", "customers", 200, 42, &[], |rows| {
            progress.store(rows, Ordering::Relaxed);
        })
        .await
        .expect("seed customers");
        assert_eq!(result.rows_inserted, 200);
        assert_eq!(result.seed_used, 42);
        assert_eq!(progress.load(Ordering::Relaxed), 200);
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM customers").await,
            "200"
        );
        // The rowid-alias primary key was left to SQLite, and the unique
        // email column really is unique.
        assert_eq!(
            scalar(&connection, "SELECT COUNT(DISTINCT id) FROM customers").await,
            "200"
        );
        assert_eq!(
            scalar(&connection, "SELECT COUNT(DISTINCT email) FROM customers").await,
            "200"
        );

        let result = seed_table(&connection, "main", "orders", 500, 7, &[], |_| {})
            .await
            .expect("seed orders");
        assert_eq!(result.rows_inserted, 500);
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM orders o
                 LEFT JOIN customers c ON c.id = o.customer_id
                 WHERE c.id IS NULL",
            )
            .await,
            "0"
        );
        // varchar(24) truncation survives the round trip.
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM orders WHERE LENGTH(note) > 24",
            )
            .await,
            "0"
        );
    }

    #[tokio::test]
    async fn the_same_seed_reproduces_the_same_rows() {
        let (_file, connection) = fixture().await;

        seed_table(&connection, "main", "customers", 50, 99, &[], |_| {})
            .await
            .expect("run 1");
        let first = column_values(&connection, "SELECT email FROM customers ORDER BY email").await;

        exec(&connection, "DELETE FROM customers").await;
        seed_table(&connection, "main", "customers", 50, 99, &[], |_| {})
            .await
            .expect("run 2");
        let second = column_values(&connection, "SELECT email FROM customers ORDER BY email").await;

        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn unique_sequences_start_above_the_rows_already_there() {
        let (_file, connection) = fixture().await;
        exec(
            &connection,
            "INSERT INTO customers (email, full_name) VALUES ('a@example.com', 'A')",
        )
        .await;

        seed_table(&connection, "main", "customers", 10, 5, &[], |_| {})
            .await
            .expect("seed");
        assert_eq!(
            scalar(&connection, "SELECT COUNT(DISTINCT email) FROM customers").await,
            "11"
        );
    }

    #[tokio::test]
    async fn per_column_specs_override_the_generator() {
        let (_file, connection) = fixture().await;
        let specs = vec![
            SeedColumnSpec {
                values: Some(vec!["Berlin".into(), "Oslo".into()]),
                null_rate: Some(0.0),
                ..spec("city")
            },
            SeedColumnSpec {
                constant: Some("30".into()),
                null_rate: Some(0.0),
                ..spec("age")
            },
        ];
        seed_table(&connection, "main", "customers", 40, 1, &specs, |_| {})
            .await
            .expect("seed with specs");

        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM customers
                 WHERE city NOT IN ('Berlin','Oslo') OR city IS NULL OR age <> 30",
            )
            .await,
            "0"
        );
    }

    #[tokio::test]
    async fn a_rejected_batch_rolls_the_whole_run_back() {
        let (_file, connection) = fixture().await;
        // A CHECK the generator cannot satisfy — `attempt`, not
        // `guarantee`, per ADR-0020. The run must leave nothing behind.
        exec(
            &connection,
            "CREATE TABLE strict_ages (
               id INTEGER PRIMARY KEY,
               label TEXT NOT NULL,
               age INTEGER NOT NULL CHECK (age < 0)
             )",
        )
        .await;

        let error = seed_table(&connection, "main", "strict_ages", 300, 3, &[], |_| {})
            .await
            .expect_err("CHECK violation must surface");
        assert!(
            error.to_lowercase().contains("check"),
            "unexpected: {error}"
        );
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM strict_ages").await,
            "0"
        );
    }
}

#[cfg(test)]
mod mysql_live_tests {
    //! End-to-end seeding against the compose MySQL
    //! (`infrastructure/test-db`, `make mysql`). Ignored by default:
    //! `cargo test --manifest-path src-tauri/Cargo.toml mysql_seed_live -- --ignored`

    use super::*;
    use crate::{MySqlStoredConnection, SshTunnelConfig, StoredConnection};

    fn test_connection() -> StoredConnection {
        StoredConnection::MySQL(MySqlStoredConnection {
            id: "seed-mysql-test".into(),
            name: "seed mysql test".into(),
            database: "dbunk_demo".into(),
            host: "127.0.0.1".into(),
            port: 13306,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            environment: crate::Environment::default(),
            safe_mode: crate::SafeMode::default(),
            read_only: false,
            last_activity_at: None,
            ssl: false,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    async fn exec(connection: &StoredConnection, sql: &str) {
        let mut conn = sqlx_connect(connection).await.expect("connect");
        sqlx::raw_sql(sql)
            .execute(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
    }

    async fn scalar(connection: &StoredConnection, sql: &str) -> String {
        let mut conn = sqlx_connect(connection).await.expect("connect");
        let rows = sqlx::query::<Any>(sql)
            .fetch_all(&mut conn)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        row_to_strings(&rows[0]).remove(0)
    }

    #[tokio::test]
    #[ignore = "requires the infrastructure/test-db mysql container"]
    async fn mysql_seed_live_end_to_end() {
        let connection = test_connection();
        exec(&connection, "DROP TABLE IF EXISTS seed_orders").await;
        exec(&connection, "DROP TABLE IF EXISTS seed_customers").await;
        exec(
            &connection,
            "CREATE TABLE seed_customers (
               id INT AUTO_INCREMENT PRIMARY KEY,
               email VARCHAR(120) NOT NULL UNIQUE,
               full_name VARCHAR(80) NOT NULL,
               city VARCHAR(60),
               tier ENUM('free','pro','team') NOT NULL,
               is_active TINYINT(1) NOT NULL,
               signed_up_at DATETIME NOT NULL
             )",
        )
        .await;
        exec(
            &connection,
            "CREATE TABLE seed_orders (
               id INT AUTO_INCREMENT PRIMARY KEY,
               customer_id INT NOT NULL,
               total_amount DECIMAL(10,2) NOT NULL,
               note VARCHAR(24),
               CONSTRAINT fk_seed_orders_customer
                 FOREIGN KEY (customer_id) REFERENCES seed_customers(id)
             )",
        )
        .await;

        // Empty parent fails before anything is written.
        let error = seed_table(
            &connection,
            "dbunk_demo",
            "seed_orders",
            10,
            42,
            &[],
            |_| {},
        )
        .await
        .expect_err("empty parent must fail");
        assert!(error.contains("seed_customers"), "unexpected: {error}");
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM seed_orders").await,
            "0"
        );

        let result = seed_table(
            &connection,
            "dbunk_demo",
            "seed_customers",
            200,
            42,
            &[],
            |_| {},
        )
        .await
        .expect("seed customers");
        assert_eq!(result.rows_inserted, 200);
        assert_eq!(
            scalar(&connection, "SELECT COUNT(*) FROM seed_customers").await,
            "200"
        );
        // AUTO_INCREMENT was left to the server, the unique index holds,
        // the ENUM only ever saw its declared members, and tinyint(1)
        // got digits rather than the word "true".
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(DISTINCT email) FROM seed_customers"
            )
            .await,
            "200"
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM seed_customers WHERE tier NOT IN ('free','pro','team')",
            )
            .await,
            "0"
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM seed_customers WHERE is_active NOT IN (0,1)",
            )
            .await,
            "0"
        );

        let result = seed_table(
            &connection,
            "dbunk_demo",
            "seed_orders",
            500,
            7,
            &[],
            |_| {},
        )
        .await
        .expect("seed orders");
        assert_eq!(result.rows_inserted, 500);
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM seed_orders o
                 LEFT JOIN seed_customers c ON c.id = o.customer_id
                 WHERE c.id IS NULL",
            )
            .await,
            "0"
        );

        exec(&connection, "DROP TABLE seed_orders").await;
        exec(&connection, "DROP TABLE seed_customers").await;
    }
}

//! Table Seeding orchestration for PostgreSQL (ADR-0020).
//!
//! Builds a [`SeedPlan`](crate::seed::SeedPlan) from introspected table
//! structure plus sampling queries (FK parent pools, unique-integer
//! MAX), generates rows with the pure generator in `crate::seed`, and
//! inserts everything inside a single transaction — a rejected batch
//! rolls back completely and surfaces the engine's error verbatim.

use std::time::Instant;

use sqlx::Acquire;

use crate::seed::{
    classify_column, generate_rows_from, insert_columns, max_char_length, parse_generator_id,
    spec_for, ColumnPlan, ColumnSource, GenKind, SeedPlan, SeedRng, DEFAULT_NULL_RATE,
};
use crate::{quote_double, ColumnInfo, SeedColumnSpec, SeedTableResult, StoredConnection};

use super::{connect, row_to_strings};

/// PostgreSQL's wire protocol caps bind parameters at 65535 per
/// statement; stay safely under it.
const MAX_PARAMS_PER_STATEMENT: usize = 60_000;

/// How many distinct parent rows to sample per foreign key.
const FK_POOL_LIMIT: usize = 1_000;

pub async fn seed_table<F>(
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

    let structure = super::fetch_table_structure(connection, schema, table).await?;
    let start = Instant::now();

    // Columns guaranteed unique by a single-column unique index or PK;
    // for composite uniques, mixing the sequence into the first member
    // is enough to make the tuple unique.
    let mut unique_columns: Vec<String> = Vec::new();
    for index in &structure.indexes {
        if index.is_unique {
            if let Some(first) = index.columns.first() {
                unique_columns.push(first.clone());
            }
        }
    }
    if let Some(pk) = &structure.primary_key {
        if let Some(first) = pk.first() {
            unique_columns.push(first.clone());
        }
    }

    // FK membership: column name -> (fk index, member position).
    let mut fk_membership: Vec<(String, usize, usize)> = Vec::new();
    for (fk_index, fk) in structure.foreign_keys.iter().enumerate() {
        for (member, column) in fk.columns.iter().enumerate() {
            fk_membership.push((column.clone(), fk_index, member));
        }
    }

    let mut plan_columns: Vec<ColumnPlan> = Vec::new();
    // Lazily sampled per FK that is actually used; index-aligned with
    // `structure.foreign_keys`, compacted into the plan afterwards.
    let mut fk_pools: Vec<Option<Vec<Vec<String>>>> = vec![None; structure.foreign_keys.len()];

    for column in &structure.columns {
        let spec = spec_for(specs, &column.name);
        let null_rate = effective_null_rate(column, spec);

        if spec.map(|s| s.skip).unwrap_or(false) {
            plan_columns.push(plain(&column.name, ColumnSource::Skip, 0.0));
            continue;
        }
        if let Some(constant) = spec.and_then(|s| s.constant.clone()) {
            plan_columns.push(plain(
                &column.name,
                ColumnSource::Constant(Some(constant)),
                null_rate,
            ));
            continue;
        }
        if let Some(values) = spec.and_then(|s| s.values.clone()) {
            if values.is_empty() {
                return Err(format!(
                    "column \"{}\" has an empty value list",
                    column.name
                ));
            }
            plan_columns.push(plain(
                &column.name,
                ColumnSource::ValueList(values),
                null_rate,
            ));
            continue;
        }

        if let Some((_, fk_index, member)) = fk_membership
            .iter()
            .find(|(name, _, _)| name == &column.name)
        {
            if null_rate >= 1.0 {
                plan_columns.push(plain(&column.name, ColumnSource::Constant(None), 0.0));
                continue;
            }
            if fk_pools[*fk_index].is_none() {
                let fk = &structure.foreign_keys[*fk_index];
                let pool = sample_fk_pool(connection, fk).await?;
                if pool.is_empty() {
                    return Err(format!(
                        "column \"{}\" references \"{}\".\"{}\", which is empty — seed \"{}\" first",
                        column.name, fk.referenced_schema, fk.referenced_table, fk.referenced_table
                    ));
                }
                fk_pools[*fk_index] = Some(pool);
            }
            plan_columns.push(plain(
                &column.name,
                ColumnSource::FkPool {
                    pool: *fk_index,
                    member: *member,
                },
                null_rate,
            ));
            continue;
        }

        // Identity / serial / defaulted-PK columns fall back to the
        // database DEFAULT, mirroring the frontend mock generator.
        let has_serial_default = column
            .default_value
            .as_deref()
            .map(|d| d.starts_with("nextval("))
            .unwrap_or(false);
        if has_serial_default || (column.is_primary_key && column.default_value.is_some()) {
            plan_columns.push(plain(&column.name, ColumnSource::Skip, 0.0));
            continue;
        }

        let kind = match spec.and_then(|s| s.generator.as_deref()) {
            Some(id) => parse_generator_id(id)?,
            None => match classify_column(&column.name, &column.data_type) {
                Some(kind) => kind,
                None if column.nullable => {
                    plan_columns.push(plain(&column.name, ColumnSource::Constant(None), 0.0));
                    continue;
                }
                None => {
                    return Err(format!(
                        "no generator for column \"{}\" of type {} — set a constant or value list",
                        column.name, column.data_type
                    ));
                }
            },
        };

        let unique = unique_columns.contains(&column.name);
        let unique_base = if unique && is_integer_kind(kind) {
            fetch_integer_max(connection, schema, table, &column.name).await?
        } else {
            0
        };

        plan_columns.push(ColumnPlan {
            name: column.name.clone(),
            source: ColumnSource::Generated {
                kind,
                unique,
                unique_base,
                min: spec.and_then(|s| s.min),
                max: spec.and_then(|s| s.max),
                max_len: max_char_length(&column.data_type),
            },
            // Unique columns skip NULL injection: a guaranteed-unique
            // column that suddenly yields NULLs is surprising even
            // where the index would allow it.
            null_rate: if unique { 0.0 } else { null_rate },
        });
    }

    let plan = SeedPlan {
        columns: plan_columns,
        fk_pools: fk_pools
            .into_iter()
            .map(|pool| pool.unwrap_or_default())
            .collect(),
        now_epoch_secs: chrono::Utc::now().timestamp(),
    };

    let columns = insert_columns(&plan);
    if columns.is_empty() {
        return Err("nothing to seed: every column is skipped".to_string());
    }
    // Generated values travel as text and are cast per-column in the
    // INSERT — sqlx declares text-bound params as TEXT, which PG
    // rejects against typed columns without an explicit cast.
    let column_types: Vec<String> = columns
        .iter()
        .map(|name| {
            structure
                .columns
                .iter()
                .find(|c| &c.name == name)
                .map(|c| c.data_type.clone())
                .unwrap_or_else(|| "text".to_string())
        })
        .collect();
    let mut rng = SeedRng::new(seed);

    let chunk_size = (MAX_PARAMS_PER_STATEMENT / columns.len()).clamp(1, 500);
    let mut conn = connect(connection).await?;
    let mut tx = conn.begin().await.map_err(|error| error.to_string())?;
    let mut rows_inserted: u64 = 0;
    let mut rows_generated: u32 = 0;
    while rows_generated < row_count {
        let batch_size = (row_count - rows_generated).min(chunk_size as u32);
        let rows = generate_rows_from(&plan, rows_generated, batch_size, &mut rng);
        let (sql, params) = build_cast_bulk_insert(schema, table, &columns, &column_types, &rows);
        let mut query = sqlx::query(&sql);
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

/// Multi-row INSERT with an explicit cast on every placeholder
/// (`$1::integer`, `$2::varchar(120)`, …). Type names come from our
/// own catalog introspection, never from user input.
fn build_cast_bulk_insert(
    schema: &str,
    table: &str,
    columns: &[String],
    column_types: &[String],
    rows: &[Vec<Option<String>>],
) -> (String, Vec<Option<String>>) {
    let qualified = format!("{}.{}", quote_double(schema), quote_double(table));
    let column_list = columns
        .iter()
        .map(|column| quote_double(column))
        .collect::<Vec<_>>()
        .join(", ");
    let mut params = Vec::with_capacity(columns.len() * rows.len());
    let values = rows
        .iter()
        .enumerate()
        .map(|(row_index, row)| {
            let placeholders = columns
                .iter()
                .enumerate()
                .map(|(column_index, _)| {
                    params.push(row.get(column_index).cloned().unwrap_or(None));
                    format!(
                        "${}::{}",
                        row_index * columns.len() + column_index + 1,
                        column_types[column_index]
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("({})", placeholders)
        })
        .collect::<Vec<_>>()
        .join(", ");
    (
        format!(
            "INSERT INTO {} ({}) VALUES {}",
            qualified, column_list, values
        ),
        params,
    )
}

fn plain(name: &str, source: ColumnSource, null_rate: f64) -> ColumnPlan {
    ColumnPlan {
        name: name.to_string(),
        source,
        null_rate,
    }
}

fn effective_null_rate(column: &ColumnInfo, spec: Option<&SeedColumnSpec>) -> f64 {
    if !column.nullable {
        return 0.0;
    }
    spec.and_then(|s| s.null_rate)
        .unwrap_or(DEFAULT_NULL_RATE)
        .clamp(0.0, 1.0)
}

fn is_integer_kind(kind: GenKind) -> bool {
    matches!(kind, GenKind::SmallInt | GenKind::Integer | GenKind::BigInt)
}

/// Sample up to [`FK_POOL_LIMIT`] distinct parent tuples for one FK.
async fn sample_fk_pool(
    connection: &StoredConnection,
    fk: &crate::ForeignKeyInfo,
) -> Result<Vec<Vec<String>>, String> {
    let columns = fk
        .referenced_columns
        .iter()
        .map(|c| quote_double(c))
        .collect::<Vec<_>>()
        .join(", ");
    let not_null = fk
        .referenced_columns
        .iter()
        .map(|c| format!("{} IS NOT NULL", quote_double(c)))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = format!(
        "SELECT DISTINCT {} FROM {}.{} WHERE {} LIMIT {}",
        columns,
        quote_double(&fk.referenced_schema),
        quote_double(&fk.referenced_table),
        not_null,
        FK_POOL_LIMIT
    );
    let mut conn = connect(connection).await?;
    let rows = sqlx::query(&sql)
        .fetch_all(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(rows.iter().map(row_to_strings).collect())
}

/// Current MAX of an integer column, so unique sequences start above
/// existing data.
async fn fetch_integer_max(
    connection: &StoredConnection,
    schema: &str,
    table: &str,
    column: &str,
) -> Result<i64, String> {
    let sql = format!(
        "SELECT COALESCE(MAX({}), 0)::bigint FROM {}.{}",
        quote_double(column),
        quote_double(schema),
        quote_double(table)
    );
    let mut conn = connect(connection).await?;
    let row = sqlx::query_scalar::<_, i64>(&sql)
        .fetch_one(&mut *conn)
        .await
        .map_err(|error| error.to_string())?;
    Ok(row)
}

#[cfg(test)]
mod live_tests {
    //! End-to-end tests against the compose Postgres
    //! (`infrastructure/test-db`, `just test-db-up postgres`).
    //! Ignored by default; run with:
    //! `cargo test --manifest-path src-tauri/Cargo.toml seed_live -- --ignored`

    use super::*;
    use crate::{PgStoredConnection, SshTunnelConfig, StoredConnection};

    fn test_connection() -> StoredConnection {
        StoredConnection::PostgreSQL(PgStoredConnection {
            id: "seed-live-test".into(),
            name: "seed live test".into(),
            database: "dbunk_demo".into(),
            host: "localhost".into(),
            port: 15432,
            user: "dbunk".into(),
            password: "dbunk".into(),
            role: "read/write".into(),
            last_activity_at: None,
            ssl: false,
            driver_options: None,
            ssh_tunnel: SshTunnelConfig::default(),
        })
    }

    async fn exec(connection: &StoredConnection, sql: &str) {
        let mut conn = connect(connection).await.expect("connect");
        sqlx::raw_sql(sql).execute(&mut *conn).await.expect(sql);
    }

    async fn count(connection: &StoredConnection, table: &str) -> i64 {
        let mut conn = connect(connection).await.expect("connect");
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM seed_live.{table}"))
            .fetch_one(&mut *conn)
            .await
            .expect("count")
    }

    #[tokio::test]
    #[ignore = "requires the infrastructure/test-db postgres container"]
    async fn seed_live_end_to_end() {
        let connection = test_connection();
        exec(&connection, "DROP SCHEMA IF EXISTS seed_live CASCADE").await;
        exec(
            &connection,
            "CREATE SCHEMA seed_live;
             CREATE TABLE seed_live.customers (
               id serial PRIMARY KEY,
               email varchar(120) NOT NULL UNIQUE,
               full_name text NOT NULL,
               city text,
               age integer CHECK (age >= 0),
               created_at timestamptz NOT NULL
             );
             CREATE TABLE seed_live.orders (
               id serial PRIMARY KEY,
               customer_id integer NOT NULL REFERENCES seed_live.customers(id),
               total_amount numeric(10,2) NOT NULL,
               note varchar(24),
               placed_on date
             );",
        )
        .await;

        // Child first must fail fast: parent is empty.
        let err = seed_table(&connection, "seed_live", "orders", 10, 42, &[], |_| {})
            .await
            .expect_err("empty parent must fail fast");
        assert!(err.contains("customers"), "unexpected error: {err}");
        assert_eq!(
            count(&connection, "orders").await,
            0,
            "must not partially apply"
        );

        // Parent seeds: unique emails, serial PK skipped, NULLs in city.
        let result = seed_table(&connection, "seed_live", "customers", 200, 42, &[], |_| {})
            .await
            .expect("seed customers");
        assert_eq!(result.rows_inserted, 200);
        assert_eq!(result.seed_used, 42);
        assert_eq!(count(&connection, "customers").await, 200);

        // Child now seeds, FK values sampled from real parent rows.
        let result = seed_table(&connection, "seed_live", "orders", 500, 7, &[], |_| {})
            .await
            .expect("seed orders");
        assert_eq!(result.rows_inserted, 500);
        let mut conn = connect(&connection).await.expect("connect");
        let orphans: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM seed_live.orders o
             LEFT JOIN seed_live.customers c ON c.id = o.customer_id
             WHERE c.id IS NULL",
        )
        .fetch_one(&mut *conn)
        .await
        .expect("orphan check");
        assert_eq!(orphans, 0);

        // Same seed on a fresh table yields identical data (determinism
        // across the full SQL round trip).
        exec(&connection, "TRUNCATE seed_live.customers CASCADE").await;
        seed_table(&connection, "seed_live", "customers", 50, 99, &[], |_| {})
            .await
            .expect("seed run 1");
        let emails_a: Vec<String> =
            sqlx::query_scalar("SELECT email FROM seed_live.customers ORDER BY email")
                .fetch_all(&mut *conn)
                .await
                .expect("emails");
        exec(&connection, "TRUNCATE seed_live.customers CASCADE").await;
        seed_table(&connection, "seed_live", "customers", 50, 99, &[], |_| {})
            .await
            .expect("seed run 2");
        let emails_b: Vec<String> =
            sqlx::query_scalar("SELECT email FROM seed_live.customers ORDER BY email")
                .fetch_all(&mut *conn)
                .await
                .expect("emails");
        assert_eq!(emails_a, emails_b);

        // Per-column spec: constant + value list + forced NULL rate.
        exec(&connection, "TRUNCATE seed_live.customers CASCADE").await;
        let specs = vec![
            SeedColumnSpec {
                column: "city".into(),
                skip: false,
                constant: None,
                values: Some(vec!["Berlin".into(), "Oslo".into()]),
                generator: None,
                min: None,
                max: None,
                null_rate: Some(0.0),
            },
            SeedColumnSpec {
                column: "age".into(),
                skip: false,
                constant: Some("30".into()),
                values: None,
                generator: None,
                min: None,
                max: None,
                null_rate: Some(0.0),
            },
        ];
        seed_table(&connection, "seed_live", "customers", 40, 1, &specs, |_| {})
            .await
            .expect("seed with specs");
        let off_spec: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM seed_live.customers
             WHERE city NOT IN ('Berlin','Oslo') OR city IS NULL OR age <> 30",
        )
        .fetch_one(&mut *conn)
        .await
        .expect("spec check");
        assert_eq!(off_spec, 0);

        exec(&connection, "DROP SCHEMA seed_live CASCADE").await;
    }
}

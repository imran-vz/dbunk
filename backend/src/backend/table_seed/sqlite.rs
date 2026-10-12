//! SQLite table seeding (ADR-0014). Same contract as PostgreSQL: a
//! dedicated connection, one transaction, parent rows and current maxima
//! read inside it, the reviewed catalog compared before and after the
//! inserts, and the shared commit fence.
//!
//! The transaction is `BEGIN IMMEDIATE`: it takes the database's write lock
//! for the whole run, so no other connection can change the schema, the
//! target or its parents until COMMIT. Unlike the interactive SQLite
//! session (which matches the sqlite3 shell), foreign keys are enforced.
//! Only the `main` database is seeded; ATTACHed databases belong to a
//! session, not to the stored connection.
use super::engine::{
    self, sqlx_failure, EngineCatalog, Execution, Failure, Identity, Plan, SeedTable, TableColumn,
};
use super::*;
use crate::{quote_double, ColumnInfo, ForeignKeyInfo, IndexInfo, StructureCapabilities};
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Connection, Executor, Row, SqliteConnection};
use std::sync::Arc;
use tokio::time::{Duration, Instant};

/// Bind variables per statement (SQLite's default limit is 32,766).
const MAX_PARAMS: usize = 32_000;
const MAX_COLUMNS: usize = 2000;
const MAX_KEYS: usize = 512;
const MAX_DEFINITION_BYTES: usize = 256 * 1024;
const MAX_PARENT_ROWS: usize = 1000;
const MAX_POOL_BYTES: usize = 4 * 1024 * 1024;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// VM instructions between cancellation checks.
const PROGRESS_OPS: i32 = 1_000;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Column {
    name: String,
    declared_type: String,
    nullable: bool,
    has_default: bool,
    /// Position in the primary key, from 1; 0 when not part of it.
    primary_key: u32,
    generated: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    name: String,
    columns: Vec<String>,
    parent_table: String,
    parent_columns: Vec<String>,
}

/// The reviewed table; `definition` is its `CREATE TABLE` text plus the
/// text of its indexes and triggers.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Catalog {
    table: String,
    definition: String,
    columns: Vec<Column>,
    unique: Vec<(String, Vec<String>)>,
    foreign_keys: Vec<ForeignKey>,
}

impl Catalog {
    fn primary_key(&self) -> Vec<String> {
        let mut keyed: Vec<&Column> = self.columns.iter().filter(|c| c.primary_key > 0).collect();
        keyed.sort_by_key(|c| c.primary_key);
        keyed.into_iter().map(|c| c.name.clone()).collect()
    }

    /// `INTEGER PRIMARY KEY` aliases the rowid, which SQLite assigns.
    fn rowid_alias(&self, column: &Column) -> bool {
        column.primary_key == 1
            && column.declared_type.eq_ignore_ascii_case("integer")
            && self.columns.iter().filter(|c| c.primary_key > 0).count() == 1
    }

    pub(crate) fn table(&self) -> SeedTable {
        let primary_key = self.primary_key();
        let structure = crate::TableStructure {
            columns: self
                .columns
                .iter()
                .enumerate()
                .map(|(index, c)| ColumnInfo {
                    name: c.name.clone(),
                    data_type: c.declared_type.clone(),
                    nullable: c.nullable,
                    default_value: c.has_default.then(|| "database default".into()),
                    is_primary_key: c.primary_key > 0,
                    ordinal_position: index as i32 + 1,
                    derivation_kind: c.generated.then(|| "generated".into()),
                })
                .collect(),
            primary_key: (!primary_key.is_empty()).then_some(primary_key),
            foreign_keys: self
                .foreign_keys
                .iter()
                .map(|fk| ForeignKeyInfo {
                    name: fk.name.clone(),
                    columns: fk.columns.clone(),
                    referenced_schema: "main".into(),
                    referenced_table: fk.parent_table.clone(),
                    referenced_columns: fk.parent_columns.clone(),
                    on_update: None,
                    on_delete: None,
                })
                .collect(),
            indexes: self
                .unique
                .iter()
                .map(|(name, columns)| IndexInfo {
                    name: name.clone(),
                    columns: columns.clone(),
                    is_unique: true,
                    is_primary: false,
                    method: None,
                })
                .collect(),
            constraints: vec![],
            triggers: vec![],
            policies: vec![],
            privileges: vec![],
            row_security: None,
            table_engine: None,
            partition_by: None,
            sample_by: None,
            capabilities: StructureCapabilities {
                columns: true,
                primary_key: true,
                foreign_keys: true,
                indexes: true,
                constraints: true,
                triggers: false,
                policies: false,
                privileges: false,
                can_insert_rows: true,
                can_update_rows: false,
                can_delete_rows: false,
                can_alter_schema: false,
                uniqueness_guarantee: "database enforced".into(),
            },
        };
        SeedTable {
            dialect: crate::seed::SeedDialect::Sqlite,
            structure,
            columns: self
                .columns
                .iter()
                .map(|c| TableColumn {
                    name: c.name.clone(),
                    data_type: c.declared_type.clone(),
                    nullable: c.nullable,
                    has_default: c.has_default,
                    generated: c.generated,
                    identity: self.rowid_alias(c),
                })
                .collect(),
            enum_labels: vec![],
        }
    }

    pub(crate) fn heap_bytes(&self) -> Option<usize> {
        use std::mem::size_of;
        let strings = |v: &[String]| {
            v.iter()
                .map(|s| s.capacity() + size_of::<String>())
                .sum::<usize>()
        };
        let mut n = self.table.capacity() + self.definition.capacity();
        for c in &self.columns {
            n += size_of::<Column>() + c.name.capacity() + c.declared_type.capacity();
        }
        for (name, columns) in &self.unique {
            n += name.capacity() + strings(columns);
        }
        for fk in &self.foreign_keys {
            n += size_of::<ForeignKey>()
                + fk.name.capacity()
                + fk.parent_table.capacity()
                + strings(&fk.columns)
                + strings(&fk.parent_columns);
        }
        Some(n)
    }
}

fn text(row: &SqliteRow, index: usize) -> Result<String, Failure> {
    row.try_get::<Option<String>, _>(index)
        .map(Option::unwrap_or_default)
        .map_err(|_| TableSeedError::Limit.into())
}

fn number(row: &SqliteRow, index: usize) -> Result<i64, Failure> {
    row.try_get::<i64, _>(index)
        .map_err(|_| TableSeedError::Limit.into())
}

/// The parent's primary key, for a foreign key that names no columns.
async fn primary_key_of(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<Vec<String>, Failure> {
    let rows = sqlx::query("SELECT name, pk FROM pragma_table_info(?) WHERE pk > 0 ORDER BY pk")
        .bind(table)
        .fetch_all(&mut *connection)
        .await
        .map_err(|e| sqlx_failure(&e))?;
    rows.iter().map(|row| text(row, 0)).collect()
}

async fn inspect_catalog(
    connection: &mut SqliteConnection,
    endpoint: &TableSeedEndpoint,
) -> Result<Catalog, Failure> {
    if endpoint.schema != "main" {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    let table = endpoint.table.as_str();
    let created =
        sqlx::query("SELECT sql FROM main.sqlite_master WHERE type = 'table' AND name = ?")
            .bind(table)
            .fetch_optional(&mut *connection)
            .await
            .map_err(|e| sqlx_failure(&e))?
            .ok_or(TableSeedError::UnsupportedTarget)?;
    let created = text(&created, 0)?;
    if created
        .trim_start()
        .to_ascii_uppercase()
        .starts_with("CREATE VIRTUAL")
    {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    // Indexes and triggers belong to the reviewed definition too.
    let dependents = sqlx::query(
        "SELECT COALESCE(sql,'') FROM main.sqlite_master \
         WHERE tbl_name = ? AND type IN ('index','trigger') ORDER BY type, name",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    let mut definition = created;
    for row in &dependents {
        definition.push('\n');
        definition.push_str(&text(row, 0)?);
    }
    if definition.len() > MAX_DEFINITION_BYTES {
        return Err(TableSeedError::Limit.into());
    }

    // `hidden` is 2 or 3 for generated columns and 1 for hidden virtual
    // table columns.
    let rows = sqlx::query(
        "SELECT name, type, \"notnull\", dflt_value IS NOT NULL, pk, hidden \
         FROM pragma_table_xinfo(?) ORDER BY cid",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    if rows.is_empty() || rows.len() > MAX_COLUMNS {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    let mut columns = Vec::with_capacity(rows.len());
    for row in &rows {
        let hidden = number(row, 5)?;
        if hidden == 1 {
            continue;
        }
        columns.push(Column {
            name: text(row, 0)?,
            declared_type: text(row, 1)?,
            nullable: number(row, 2)? == 0,
            has_default: row.try_get::<bool, _>(3).unwrap_or(false),
            primary_key: number(row, 4)?.clamp(0, i64::from(u32::MAX)) as u32,
            generated: matches!(hidden, 2 | 3),
        });
    }

    let index_rows = sqlx::query(
        "SELECT l.name, i.name FROM pragma_index_list(?) AS l, pragma_index_info(l.name) AS i \
         WHERE l.\"unique\" = 1 AND i.name IS NOT NULL ORDER BY l.name, i.seqno",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    let mut unique: Vec<(String, Vec<String>)> = Vec::new();
    for row in &index_rows {
        let (index, column) = (text(row, 0)?, text(row, 1)?);
        match unique.iter_mut().find(|(name, _)| *name == index) {
            Some((_, columns)) => columns.push(column),
            None => unique.push((index, vec![column])),
        }
    }
    if unique.len() > MAX_KEYS {
        return Err(TableSeedError::Limit.into());
    }

    let fk_rows = sqlx::query(
        "SELECT id, \"table\", \"from\", \"to\" FROM pragma_foreign_key_list(?) ORDER BY id, seq",
    )
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    let mut foreign_keys: Vec<(i64, ForeignKey, bool)> = Vec::new();
    for row in &fk_rows {
        let id = number(row, 0)?;
        let to = row.try_get::<Option<String>, _>(3).ok().flatten();
        let (column, implicit) = (text(row, 2)?, to.is_none());
        match foreign_keys.iter_mut().find(|(key, _, _)| *key == id) {
            Some((_, fk, _)) => {
                fk.columns.push(column);
                fk.parent_columns.extend(to);
            }
            None => foreign_keys.push((
                id,
                ForeignKey {
                    name: format!("fk_{table}_{id}"),
                    columns: vec![column],
                    parent_table: text(row, 1)?,
                    parent_columns: to.into_iter().collect(),
                },
                implicit,
            )),
        }
    }
    if foreign_keys.len() > MAX_KEYS {
        return Err(TableSeedError::Limit.into());
    }
    let mut resolved = Vec::with_capacity(foreign_keys.len());
    for (_, mut fk, implicit) in foreign_keys {
        if implicit {
            fk.parent_columns = primary_key_of(connection, &fk.parent_table).await?;
        }
        if fk.parent_columns.len() != fk.columns.len() {
            return Err(TableSeedError::UnsupportedTarget.into());
        }
        resolved.push(fk);
    }
    Ok(Catalog {
        table: table.to_owned(),
        definition,
        columns,
        unique,
        foreign_keys: resolved,
    })
}

async fn connect(
    stored: &crate::SqliteStoredConnection,
    control: &Arc<Control>,
) -> Result<SqliteConnection, Failure> {
    if stored.database.trim().is_empty() || stored.database.contains(":memory:") {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    crate::backend::development::connections::engine_connections::check_sqlite_file(
        &stored.database,
    )
    .await
    .map_err(|_| TableSeedError::UnsupportedTarget)?;
    let options = SqliteConnectOptions::new()
        .filename(&stored.database)
        .create_if_missing(false)
        .read_only(stored.read_only)
        .busy_timeout(BUSY_TIMEOUT)
        .foreign_keys(true)
        .statement_cache_capacity(16);
    let mut connection = SqliteConnection::connect_with(&options)
        .await
        .map_err(|e| sqlx_failure(&e))?;
    // A cancelled job interrupts whatever statement is running.
    let watched = control.clone();
    let installed = match connection.lock_handle().await {
        Ok(mut handle) => {
            handle.set_progress_handler(PROGRESS_OPS, move || !watched.is_cancelled());
            Ok(())
        }
        Err(error) => Err(sqlx_failure(&error)),
    };
    if let Err(failure) = installed {
        let _ = connection.close().await;
        return Err(failure);
    }
    Ok(connection)
}

fn target_of(plan: &Plan) -> Result<&Catalog, Failure> {
    match &plan.catalog {
        EngineCatalog::Sqlite(catalog) => Ok(catalog),
        _ => Err(TableSeedError::UnsupportedTarget.into()),
    }
}

pub(crate) async fn inspect(
    stored: &crate::SqliteStoredConnection,
    intent: TableSeedIntent,
    target: TableSeedConnection,
    control: &Arc<Control>,
) -> Result<Plan, Failure> {
    let deadline = Instant::now() + engine::INSPECTION_DEADLINE;
    let operation = async {
        let mut connection = connect(stored, control).await?;
        let result = async {
            connection
                .execute("BEGIN")
                .await
                .map_err(|e| sqlx_failure(&e))?;
            inspect_catalog(&mut connection, &intent.endpoint).await
        }
        .await;
        let _ = connection.close().await;
        let catalog = result?;
        let identity = Identity::Fingerprint(engine::fingerprint(&catalog.definition));
        Plan::build(EngineCatalog::Sqlite(catalog), intent, target, identity)
    };
    tokio::select! {
        biased;
        _ = control.cancelled() => Err(TableSeedError::Cancelled.into()),
        _ = tokio::time::sleep_until(deadline) => Err(TableSeedError::Timeout.into()),
        r = operation => r,
    }
}

/// Parent pools and unique maxima, read under the write lock.
async fn finalize(
    connection: &mut SqliteConnection,
    plan: &Plan,
    catalog: &Catalog,
) -> Result<crate::seed::SeedPlan, Failure> {
    let table = catalog.table();
    let draft = engine::draft(&table, &plan.intent)?;
    let mut pools = vec![None; table.structure.foreign_keys.len()];
    let mut bytes = 0usize;
    for index in &draft.needed_pools {
        let fk = &table.structure.foreign_keys[*index];
        let names: Vec<String> = fk
            .referenced_columns
            .iter()
            .map(|c| quote_double(c))
            .collect();
        let sql = format!(
            "SELECT DISTINCT {} FROM main.{} WHERE {} LIMIT {MAX_PARENT_ROWS}",
            names
                .iter()
                .map(|n| format!("CAST({n} AS TEXT)"))
                .collect::<Vec<_>>()
                .join(","),
            quote_double(&fk.referenced_table),
            names
                .iter()
                .map(|n| format!("{n} IS NOT NULL"))
                .collect::<Vec<_>>()
                .join(" AND ")
        );
        let rows = connection
            .fetch_all(sql.as_str())
            .await
            .map_err(|e| sqlx_failure(&e))?;
        let mut sampled = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut values = Vec::with_capacity(names.len());
            for i in 0..names.len() {
                let value = text(row, i)?;
                if value.len() > 8192 {
                    return Err(TableSeedError::Limit.into());
                }
                bytes = bytes.saturating_add(value.capacity() + std::mem::size_of::<String>());
                if bytes > MAX_POOL_BYTES {
                    return Err(TableSeedError::Limit.into());
                }
                values.push(value);
            }
            sampled.push(values);
        }
        if sampled.is_empty() {
            return Err(Failure::empty_parent(fk));
        }
        pools[*index] = Some(sampled);
    }
    let mut maxes = Vec::new();
    for name in &draft.needed_maxes {
        let sql = format!(
            "SELECT CAST(COALESCE(MAX({}),0) AS INTEGER) FROM main.{}",
            quote_double(name),
            quote_double(&catalog.table)
        );
        let value = number(
            &connection
                .fetch_one(sql.as_str())
                .await
                .map_err(|e| sqlx_failure(&e))?,
            0,
        )?;
        value
            .checked_add(i64::from(plan.intent.row_count))
            .ok_or(TableSeedError::IntegerRange)?;
        maxes.push((name.clone(), value));
    }
    engine::capacity(&draft, plan.intent.row_count, &plan.intent)?;
    let mut seed_plan =
        crate::seed::finalize_plan(&table.structure, draft, pools, &maxes, plan.clock)
            .map_err(|_| TableSeedError::InvalidRecipe)?;
    seed_plan.options = engine::options(&plan.intent);
    Ok(seed_plan)
}

pub(crate) async fn execute(
    stored: &crate::SqliteStoredConnection,
    plan: Arc<Plan>,
    control: &Arc<Control>,
) -> Execution {
    let deadline = engine::deadline();
    let mut connection: Option<SqliteConnection> = None;
    let mut began = false;
    let operation = async {
        let catalog = target_of(&plan)?;
        connection = Some(connect(stored, control).await?);
        let c = connection.as_mut().unwrap();
        c.execute("BEGIN IMMEDIATE")
            .await
            .map_err(|e| sqlx_failure(&e))?;
        began = true;
        if &inspect_catalog(c, &plan.intent.endpoint).await? != catalog {
            return Err(TableSeedError::TargetChanged.into());
        }
        let seed = finalize(c, &plan, catalog).await?;
        let names = crate::seed::insert_columns(&seed);
        let prefix = format!(
            "INSERT INTO main.{} ({}) VALUES ",
            quote_double(&catalog.table),
            names
                .iter()
                .map(|n| quote_double(n))
                .collect::<Vec<_>>()
                .join(",")
        );
        let placeholders = vec![1usize; names.len()];
        let batch = engine::batch_size(&seed, &placeholders, prefix.len(), MAX_PARAMS)?;
        let mut rng = crate::seed::SeedRng::new(plan.seed_used);
        let (mut offset, mut inserted) = (0u32, 0u64);
        while offset < plan.intent.row_count {
            if control.is_cancelled() {
                return Err(TableSeedError::Cancelled.into());
            }
            let count = batch.min(plan.intent.row_count - offset);
            let rows = crate::seed::generate_rows_from(&seed, offset, count, &mut rng);
            let row = format!("({})", vec!["?"; names.len()].join(","));
            let sql = format!("{prefix}{}", vec![row.as_str(); rows.len()].join(","));
            if sql.len() > 1024 * 1024 {
                return Err(TableSeedError::Limit.into());
            }
            let mut query = sqlx::query(&sql);
            for value in rows.iter().flatten() {
                query = query.bind(value.as_deref());
            }
            inserted += query
                .execute(&mut *c)
                .await
                .map_err(|e| sqlx_failure(&e))?
                .rows_affected();
            offset += count;
            control.progress(u64::from(offset));
        }
        if &inspect_catalog(c, &plan.intent.endpoint).await? != catalog {
            return Err(TableSeedError::TargetChanged.into());
        }
        Ok::<u64, Failure>(inserted)
    };
    let ready = tokio::select! {
        biased;
        _ = control.cancelled() => Err(TableSeedError::Cancelled.into()),
        _ = tokio::time::sleep_until(deadline) => Err(TableSeedError::Timeout.into()),
        r = operation => r,
    };
    let execution = match ready {
        Ok(rows) => {
            let c = connection.as_mut().unwrap();
            engine::commit_result(
                async {
                    c.execute("COMMIT")
                        .await
                        .map(drop)
                        .map_err(|e| sqlx_failure(&e))
                },
                control,
                rows,
                deadline,
            )
            .await
        }
        Err(error) => Execution::failed(began, error),
    };
    // Closing without COMMIT rolls back; the progress handler has already
    // interrupted any statement a cancel left running.
    if let Some(mut c) = connection {
        if !matches!(execution.outcome, TableSeedOutcome::Completed { .. }) {
            let _ = tokio::time::timeout(Duration::from_secs(1), c.execute("ROLLBACK")).await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), c.close()).await;
    }
    execution
}

#[cfg(test)]
mod tests;

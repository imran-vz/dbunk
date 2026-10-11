//! MySQL table seeding (ADR-0014). Same contract as PostgreSQL: one
//! dedicated connection, one InnoDB transaction, parent rows and current
//! maxima read inside it, the reviewed catalog compared before and after the
//! inserts, and the shared commit fence. Tables without transactions are
//! refused, because a failed seed could not be rolled back.
//!
//! Locking: reading the target first takes InnoDB's metadata lock, which
//! holds off concurrent DDL until COMMIT. Unique maxima are read `FOR
//! UPDATE`, locking the gap above them against concurrent inserts, and
//! sampled parent rows `LOCK IN SHARE MODE`, so they cannot be deleted
//! before the seed commits.
use super::engine::{
    self, sqlx_failure, EngineCatalog, Execution, Failure, Identity, Plan, SeedTable, TableColumn,
};
use super::*;
use crate::{quote_backtick, ColumnInfo, ForeignKeyInfo, IndexInfo, StructureCapabilities};
use sqlx::mysql::{MySqlConnectOptions, MySqlConnection, MySqlRow};
use sqlx::{ConnectOptions, Connection, Executor, Row};
use tokio::time::{Duration, Instant};

/// Placeholders per statement (the protocol allows 65,535).
const MAX_PARAMS: usize = 60_000;
/// Columns, keys and definition text a catalog may hold.
const MAX_COLUMNS: usize = 4096;
const MAX_KEYS: usize = 512;
const MAX_DEFINITION_BYTES: usize = 256 * 1024;
/// Parent rows sampled per foreign key, and their total size.
const MAX_PARENT_ROWS: usize = 1000;
const MAX_POOL_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Column {
    name: String,
    column_type: String,
    nullable: bool,
    has_default: bool,
    auto_increment: bool,
    generated: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ForeignKey {
    name: String,
    columns: Vec<String>,
    parent_schema: String,
    parent_table: String,
    parent_columns: Vec<String>,
}

/// The reviewed table. `definition` is `SHOW CREATE TABLE` without its
/// `AUTO_INCREMENT=` counter, which the seed's own inserts advance.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Catalog {
    database: String,
    table: String,
    definition: String,
    columns: Vec<Column>,
    primary_key: Vec<String>,
    unique: Vec<(String, Vec<String>)>,
    foreign_keys: Vec<ForeignKey>,
}

impl Catalog {
    pub(crate) fn table(&self) -> SeedTable {
        let structure = crate::TableStructure {
            columns: self
                .columns
                .iter()
                .enumerate()
                .map(|(index, c)| ColumnInfo {
                    name: c.name.clone(),
                    data_type: c.column_type.clone(),
                    nullable: c.nullable,
                    default_value: if c.auto_increment {
                        Some("auto_increment".into())
                    } else {
                        c.has_default.then(|| "database default".into())
                    },
                    is_primary_key: self.primary_key.contains(&c.name),
                    ordinal_position: index as i32 + 1,
                    derivation_kind: c.generated.then(|| "generated".into()),
                })
                .collect(),
            primary_key: (!self.primary_key.is_empty()).then(|| self.primary_key.clone()),
            foreign_keys: self
                .foreign_keys
                .iter()
                .map(|fk| ForeignKeyInfo {
                    name: fk.name.clone(),
                    columns: fk.columns.clone(),
                    referenced_schema: fk.parent_schema.clone(),
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
                    is_primary: name == "PRIMARY",
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
            dialect: crate::seed::SeedDialect::MySql,
            structure,
            columns: self
                .columns
                .iter()
                .map(|c| TableColumn {
                    name: c.name.clone(),
                    data_type: c.column_type.clone(),
                    nullable: c.nullable,
                    has_default: c.has_default || c.auto_increment,
                    generated: c.generated,
                    identity: c.auto_increment,
                })
                .collect(),
            enum_labels: vec![],
        }
    }

    pub(crate) fn heap_bytes(&self) -> Option<usize> {
        use std::mem::size_of;
        let strings = |v: &[String]| v.iter().map(|s| s.capacity() + size_of::<String>()).sum::<usize>();
        let mut n = self.database.capacity() + self.table.capacity() + self.definition.capacity();
        for c in &self.columns {
            n += size_of::<Column>() + c.name.capacity() + c.column_type.capacity();
        }
        n += strings(&self.primary_key);
        for (name, columns) in &self.unique {
            n += name.capacity() + strings(columns);
        }
        for fk in &self.foreign_keys {
            n += size_of::<ForeignKey>()
                + fk.name.capacity()
                + fk.parent_schema.capacity()
                + fk.parent_table.capacity()
                + strings(&fk.columns)
                + strings(&fk.parent_columns);
        }
        Some(n)
    }
}

fn qualified(database: &str, table: &str) -> String {
    format!("{}.{}", quote_backtick(database), quote_backtick(table))
}

/// `SHOW CREATE TABLE` without the auto-increment counter.
fn stable_definition(definition: &str) -> String {
    let mut out = String::with_capacity(definition.len());
    let mut rest = definition;
    while let Some(at) = rest.find("AUTO_INCREMENT=") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + "AUTO_INCREMENT=".len()..];
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        rest = tail[digits..].trim_start_matches(' ');
    }
    out.push_str(rest);
    out
}

fn text(row: &MySqlRow, index: usize) -> Result<String, Failure> {
    // information_schema text can arrive as VARBINARY on some servers.
    row.try_get::<String, _>(index)
        .or_else(|_| {
            row.try_get::<Vec<u8>, _>(index)
                .map(|b| String::from_utf8_lossy(&b).into_owned())
        })
        .map_err(|_| TableSeedError::Limit.into())
}

fn optional_text(row: &MySqlRow, index: usize) -> Result<Option<String>, Failure> {
    match row.try_get::<Option<String>, _>(index) {
        Ok(value) => Ok(value),
        Err(_) => row
            .try_get::<Option<Vec<u8>>, _>(index)
            .map(|b| b.map(|b| String::from_utf8_lossy(&b).into_owned()))
            .map_err(|_| TableSeedError::Limit.into()),
    }
}

/// Grouped `(name, column)` rows in their listed order.
fn group(rows: Vec<(String, String)>) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for (name, column) in rows {
        match grouped.iter_mut().find(|(n, _)| *n == name) {
            Some((_, columns)) => columns.push(column),
            None => grouped.push((name, vec![column])),
        }
    }
    grouped
}

async fn inspect_catalog(
    connection: &mut MySqlConnection,
    endpoint: &TableSeedEndpoint,
) -> Result<Catalog, Failure> {
    let (database, table) = (endpoint.schema.as_str(), endpoint.table.as_str());
    let kind = sqlx::query(
        "SELECT CAST(table_type AS CHAR), CAST(COALESCE(engine,'') AS CHAR) \
         FROM information_schema.tables WHERE table_schema = ? AND table_name = ?",
    )
    .bind(database)
    .bind(table)
    .fetch_optional(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?
    .ok_or(TableSeedError::UnsupportedTarget)?;
    let (table_type, engine_name) = (text(&kind, 0)?, text(&kind, 1)?);
    if table_type != "BASE TABLE" || !engine_name.eq_ignore_ascii_case("InnoDB") {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    let created = connection
        .fetch_one(format!("SHOW CREATE TABLE {}", qualified(database, table)).as_str())
        .await
        .map_err(|e| sqlx_failure(&e))?;
    let definition = stable_definition(&text(&created, 1)?);
    if definition.len() > MAX_DEFINITION_BYTES {
        return Err(TableSeedError::Limit.into());
    }

    let rows = sqlx::query(
        "SELECT CAST(column_name AS CHAR), CAST(column_type AS CHAR), \
                CAST(is_nullable AS CHAR), column_default IS NOT NULL, \
                CAST(extra AS CHAR), CAST(COALESCE(generation_expression,'') AS CHAR) \
         FROM information_schema.columns \
         WHERE table_schema = ? AND table_name = ? ORDER BY ordinal_position",
    )
    .bind(database)
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    if rows.is_empty() || rows.len() > MAX_COLUMNS {
        return Err(TableSeedError::UnsupportedTarget.into());
    }
    let mut columns = Vec::with_capacity(rows.len());
    for row in &rows {
        let extra = text(row, 4)?.to_ascii_lowercase();
        let name = text(row, 0)?;
        if name.is_empty() || name.len() > 64 {
            return Err(TableSeedError::UnsupportedTarget.into());
        }
        columns.push(Column {
            name,
            column_type: text(row, 1)?.to_ascii_lowercase(),
            nullable: text(row, 2)? == "YES",
            has_default: row.try_get::<bool, _>(3).unwrap_or(false),
            auto_increment: extra.contains("auto_increment"),
            generated: !text(row, 5)?.is_empty(),
        });
    }

    let index_rows = sqlx::query(
        "SELECT CAST(index_name AS CHAR), CAST(column_name AS CHAR) \
         FROM information_schema.statistics \
         WHERE table_schema = ? AND table_name = ? AND non_unique = 0 \
         ORDER BY index_name, seq_in_index",
    )
    .bind(database)
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    let mut pairs = Vec::with_capacity(index_rows.len());
    for row in &index_rows {
        // Expression parts of functional indexes have no column name.
        match optional_text(row, 1)? {
            Some(column) => pairs.push((text(row, 0)?, column)),
            None => continue,
        }
    }
    let unique = group(pairs);
    if unique.len() > MAX_KEYS {
        return Err(TableSeedError::Limit.into());
    }
    let primary_key = unique
        .iter()
        .find(|(name, _)| name == "PRIMARY")
        .map(|(_, columns)| columns.clone())
        .unwrap_or_default();

    let fk_rows = sqlx::query(
        "SELECT CAST(constraint_name AS CHAR), CAST(column_name AS CHAR), \
                CAST(referenced_table_schema AS CHAR), CAST(referenced_table_name AS CHAR), \
                CAST(referenced_column_name AS CHAR) \
         FROM information_schema.key_column_usage \
         WHERE table_schema = ? AND table_name = ? AND referenced_table_name IS NOT NULL \
         ORDER BY constraint_name, ordinal_position",
    )
    .bind(database)
    .bind(table)
    .fetch_all(&mut *connection)
    .await
    .map_err(|e| sqlx_failure(&e))?;
    let mut foreign_keys: Vec<ForeignKey> = Vec::new();
    for row in &fk_rows {
        let name = text(row, 0)?;
        let (column, parent_column) = (text(row, 1)?, text(row, 4)?);
        match foreign_keys.iter_mut().find(|fk| fk.name == name) {
            Some(fk) => {
                fk.columns.push(column);
                fk.parent_columns.push(parent_column);
            }
            None => foreign_keys.push(ForeignKey {
                name,
                columns: vec![column],
                parent_schema: text(row, 2)?,
                parent_table: text(row, 3)?,
                parent_columns: vec![parent_column],
            }),
        }
    }
    if foreign_keys.len() > MAX_KEYS {
        return Err(TableSeedError::Limit.into());
    }
    Ok(Catalog {
        database: database.to_owned(),
        table: table.to_owned(),
        definition,
        columns,
        primary_key,
        unique,
        foreign_keys,
    })
}

/// A dedicated connection with the job's session settings: UTC, strict
/// type checking and bounded lock waits.
struct Session {
    connection: MySqlConnection,
    options: MySqlConnectOptions,
    thread: u64,
    _route: crate::backend::bastions::ProbeRoute,
}

async fn connect(
    inner: &crate::backend::Inner,
    stored: &crate::MySqlStoredConnection,
    deadline: Instant,
) -> Result<Session, Failure> {
    let (route, resolved) = crate::backend::bastions::route_probe(
        &inner.state,
        crate::StoredConnection::MySQL(stored.clone()),
        deadline,
    )
    .await
    .map_err(|_| TableSeedError::Database)?;
    let crate::StoredConnection::MySQL(resolved) = &resolved else {
        return Err(TableSeedError::UnsupportedTarget.into());
    };
    let options = crate::backend::mysql_sessions::connect_options(resolved)
        .map_err(|_| TableSeedError::UnsupportedTarget)?;
    let mut connection = tokio::time::timeout_at(deadline, options.connect())
        .await
        .map_err(|_| TableSeedError::Timeout)?
        .map_err(|e| sqlx_failure(&e))?;
    let setup = async {
        connection
            .execute(
                "SET SESSION time_zone = '+00:00', \
                 SESSION sql_mode = 'STRICT_ALL_TABLES,NO_ENGINE_SUBSTITUTION,\
                 ERROR_FOR_DIVISION_BY_ZERO,NO_ZERO_DATE,NO_ZERO_IN_DATE', \
                 SESSION innodb_lock_wait_timeout = 10",
            )
            .await?;
        connection.fetch_one("SELECT CONNECTION_ID()").await
    };
    let row = match setup.await {
        Ok(row) => row,
        Err(error) => {
            let _ = connection.close().await;
            return Err(sqlx_failure(&error));
        }
    };
    let thread = row
        .try_get::<u64, _>(0)
        .or_else(|_| row.try_get::<i64, _>(0).map(|t| t as u64))
        .unwrap_or_default();
    Ok(Session {
        connection,
        options,
        thread,
        _route: route,
    })
}

/// Stops the statement running on `thread` from a short side connection.
async fn kill(options: &MySqlConnectOptions, thread: u64) {
    if thread == 0 {
        return;
    }
    let attempt = async {
        let mut side = options.connect().await?;
        side.execute(format!("KILL QUERY {thread}").as_str())
            .await?;
        side.close().await
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), attempt).await;
}

fn target_of(plan: &Plan) -> Result<&Catalog, Failure> {
    match &plan.catalog {
        EngineCatalog::MySql(catalog) => Ok(catalog),
        _ => Err(TableSeedError::UnsupportedTarget.into()),
    }
}

pub(crate) async fn inspect(
    inner: &crate::backend::Inner,
    stored: &crate::MySqlStoredConnection,
    intent: TableSeedIntent,
    target: TableSeedConnection,
    control: &Control,
) -> Result<Plan, Failure> {
    let deadline = Instant::now() + engine::INSPECTION_DEADLINE;
    let operation = async {
        let mut session = connect(inner, stored, deadline).await?;
        let result = async {
            session
                .connection
                .execute("START TRANSACTION READ ONLY")
                .await
                .map_err(|e| sqlx_failure(&e))?;
            inspect_catalog(&mut session.connection, &intent.endpoint).await
        }
        .await;
        let _ = session.connection.close().await;
        let catalog = result?;
        let identity = Identity::Fingerprint(engine::fingerprint(&catalog.definition));
        Plan::build(EngineCatalog::MySql(catalog), intent, target, identity)
    };
    tokio::select! {
        biased;
        _ = control.cancelled() => Err(TableSeedError::Cancelled.into()),
        _ = tokio::time::sleep_until(deadline) => Err(TableSeedError::Timeout.into()),
        r = operation => r,
    }
}

/// Parent pools and unique maxima, read inside the write transaction.
async fn finalize(
    connection: &mut MySqlConnection,
    plan: &Plan,
    catalog: &Catalog,
) -> Result<crate::seed::SeedPlan, Failure> {
    let table = catalog.table();
    let draft = engine::draft(&table, &plan.intent)?;
    let mut pools = vec![None; table.structure.foreign_keys.len()];
    let mut bytes = 0usize;
    for index in &draft.needed_pools {
        let fk = &table.structure.foreign_keys[*index];
        let parent = qualified(&fk.referenced_schema, &fk.referenced_table);
        let names: Vec<String> = fk.referenced_columns.iter().map(|c| quote_backtick(c)).collect();
        let sql = format!(
            "SELECT DISTINCT {} FROM {parent} WHERE {} LIMIT {MAX_PARENT_ROWS} LOCK IN SHARE MODE",
            names
                .iter()
                .map(|n| format!("CAST({n} AS CHAR)"))
                .collect::<Vec<_>>()
                .join(","),
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
            "SELECT CAST(COALESCE(MAX({}),0) AS SIGNED) FROM {} FOR UPDATE",
            quote_backtick(name),
            qualified(&catalog.database, &catalog.table)
        );
        let value: i64 = connection
            .fetch_one(sql.as_str())
            .await
            .map_err(|e| sqlx_failure(&e))?
            .try_get(0)
            .map_err(|_| TableSeedError::IntegerRange)?;
        let column = catalog
            .columns
            .iter()
            .find(|c| &c.name == name)
            .ok_or(TableSeedError::TargetChanged)?;
        let end = value
            .checked_add(i64::from(plan.intent.row_count))
            .ok_or(TableSeedError::IntegerRange)?;
        if end > integer_max(&column.column_type) {
            return Err(TableSeedError::IntegerRange.into());
        }
        maxes.push((name.clone(), value));
    }
    engine::capacity(&draft, plan.intent.row_count, &plan.intent)?;
    let mut seed_plan =
        crate::seed::finalize_plan(&table.structure, draft, pools, &maxes, plan.clock)
            .map_err(|_| TableSeedError::InvalidRecipe)?;
    seed_plan.options = engine::options(&plan.intent);
    Ok(seed_plan)
}

/// The largest value of a MySQL integer column type.
fn integer_max(column_type: &str) -> i64 {
    let unsigned = column_type.contains("unsigned");
    let base = column_type.split(['(', ' ']).next().unwrap_or("");
    match (base, unsigned) {
        ("tinyint", false) => i64::from(i8::MAX),
        ("tinyint", true) => i64::from(u8::MAX),
        ("smallint", false) => i64::from(i16::MAX),
        ("smallint", true) => i64::from(u16::MAX),
        ("mediumint", false) => 8_388_607,
        ("mediumint", true) => 16_777_215,
        ("int" | "integer", false) => i64::from(i32::MAX),
        ("int" | "integer", true) => i64::from(u32::MAX),
        _ => i64::MAX,
    }
}

pub(crate) async fn execute(
    inner: &crate::backend::Inner,
    stored: &crate::MySqlStoredConnection,
    plan: Arc<Plan>,
    control: &Control,
) -> Execution {
    let deadline = engine::deadline();
    let mut session: Option<Session> = None;
    let mut began = false;
    let operation = async {
        let catalog = target_of(&plan)?;
        session = Some(connect(inner, stored, deadline).await?);
        let connection = &mut session.as_mut().unwrap().connection;
        connection
            .execute("START TRANSACTION")
            .await
            .map_err(|e| sqlx_failure(&e))?;
        began = true;
        let target = qualified(&catalog.database, &catalog.table);
        // Takes the metadata lock: no DDL on the target until COMMIT.
        connection
            .execute(format!("SELECT 1 FROM {target} LIMIT 0").as_str())
            .await
            .map_err(|e| sqlx_failure(&e))?;
        if &inspect_catalog(connection, &plan.intent.endpoint).await? != catalog {
            return Err(TableSeedError::TargetChanged.into());
        }
        let seed = finalize(connection, &plan, catalog).await?;
        let names = crate::seed::insert_columns(&seed);
        let prefix = format!(
            "INSERT INTO {target} ({}) VALUES ",
            names
                .iter()
                .map(|n| quote_backtick(n))
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
                .execute(&mut *connection)
                .await
                .map_err(|e| sqlx_failure(&e))?
                .rows_affected();
            offset += count;
            control.progress(u64::from(offset));
        }
        if &inspect_catalog(connection, &plan.intent.endpoint).await? != catalog {
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
            let connection = &mut session.as_mut().unwrap().connection;
            engine::commit_result(
                async {
                    connection
                        .execute("COMMIT")
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
    // Closing without COMMIT rolls the transaction back on the server. A
    // statement interrupted by cancel or the deadline is stopped first.
    if let Some(session) = session {
        if !matches!(execution.outcome, TableSeedOutcome::Completed { .. }) {
            kill(&session.options, session.thread).await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), session.connection.close()).await;
    }
    execution
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_increment_counters_do_not_change_the_definition() {
        let before = "CREATE TABLE `t` (\n  `id` int NOT NULL AUTO_INCREMENT\n) ENGINE=InnoDB AUTO_INCREMENT=7 DEFAULT CHARSET=utf8mb4";
        let after = "CREATE TABLE `t` (\n  `id` int NOT NULL AUTO_INCREMENT\n) ENGINE=InnoDB AUTO_INCREMENT=107 DEFAULT CHARSET=utf8mb4";
        assert_eq!(stable_definition(before), stable_definition(after));
        assert!(stable_definition(before).contains("NOT NULL AUTO_INCREMENT"));
        assert!(!stable_definition(before).contains("AUTO_INCREMENT=7"));
    }

    #[test]
    fn integer_limits_follow_the_column_type() {
        assert_eq!(integer_max("int(11)"), i64::from(i32::MAX));
        assert_eq!(integer_max("int unsigned"), i64::from(u32::MAX));
        assert_eq!(integer_max("smallint(5) unsigned"), i64::from(u16::MAX));
        assert_eq!(integer_max("bigint"), i64::MAX);
    }

    #[test]
    fn catalogs_become_seed_tables() {
        let catalog = Catalog {
            database: "shop".into(),
            table: "orders".into(),
            definition: String::new(),
            columns: vec![
                Column {
                    name: "id".into(),
                    column_type: "int".into(),
                    nullable: false,
                    has_default: false,
                    auto_increment: true,
                    generated: false,
                },
                Column {
                    name: "status".into(),
                    column_type: "enum('new','paid')".into(),
                    nullable: false,
                    has_default: false,
                    auto_increment: false,
                    generated: false,
                },
                Column {
                    name: "total".into(),
                    column_type: "decimal(8,2)".into(),
                    nullable: true,
                    has_default: false,
                    auto_increment: false,
                    generated: true,
                },
            ],
            primary_key: vec!["id".into()],
            unique: vec![("PRIMARY".into(), vec!["id".into()])],
            foreign_keys: vec![],
        };
        let table = catalog.table();
        let intent = TableSeedIntent::new(
            TableSeedEndpoint {
                connection_id: "c".into(),
                schema: "shop".into(),
                table: "orders".into(),
            },
            10,
            Some(1),
            vec![],
        )
        .unwrap();
        let draft = engine::draft(&table, &intent).expect("draft");
        let columns = engine::columns(&table, Some(&draft));
        assert_eq!(columns[0].action, TableSeedColumnAction::Default, "auto_increment");
        assert_eq!(columns[1].action, TableSeedColumnAction::Values, "enum members");
        assert_eq!(columns[2].action, TableSeedColumnAction::Default, "generated");
    }
}

//! Engine-neutral seed planning, preview and commit (ADR-0014). Each engine
//! runner inspects its own catalog into an [`EngineCatalog`], and everything
//! that decides what to generate is shared through its [`SeedTable`] view.
//! Only reading parent rows, current maxima and the inserts themselves are
//! engine-specific.
use super::*;
use crate::seed::{self, ColumnSource, SeedDialect};
use crate::TableStructure;
use tokio::time::{Duration, Instant};

/// Seed used to preview a recipe that leaves its seed to review.
const PREVIEW_SEED: u64 = 1;

/// One column as the planner needs it.
pub(crate) struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub has_default: bool,
    /// Computed by the database (`GENERATED ... AS`).
    pub generated: bool,
    /// Numbered by the database (identity, serial, AUTO_INCREMENT, rowid).
    pub identity: bool,
}

/// An inspected table in engine-neutral form.
pub(crate) struct SeedTable {
    pub dialect: SeedDialect,
    pub structure: TableStructure,
    pub columns: Vec<TableColumn>,
    /// Declared labels of PostgreSQL enum columns. MySQL enums travel in the
    /// column type and are expanded by the generator.
    pub enum_labels: Vec<(String, Vec<String>)>,
}

/// An inspected catalog, compared again inside the write transaction to
/// detect a changed target.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum EngineCatalog {
    Postgres(crate::postgres::native_table_seed::Catalog),
    MySql(super::mysql::Catalog),
    Sqlite(super::sqlite::Catalog),
}

impl EngineCatalog {
    pub(crate) fn table(&self) -> SeedTable {
        match self {
            EngineCatalog::Postgres(catalog) => catalog.table(),
            EngineCatalog::MySql(catalog) => catalog.table(),
            EngineCatalog::Sqlite(catalog) => catalog.table(),
        }
    }

    fn heap_bytes(&self) -> Option<usize> {
        match self {
            EngineCatalog::Postgres(catalog) => catalog.heap_bytes(),
            EngineCatalog::MySql(catalog) => catalog.heap_bytes(),
            EngineCatalog::Sqlite(catalog) => catalog.heap_bytes(),
        }
    }
}

pub(crate) struct Plan {
    pub description: Option<TableSeedDescription>,
    pub intent: TableSeedIntent,
    pub columns: Vec<TableSeedColumn>,
    pub issue: Option<TableSeedError>,
    pub seed_used: u64,
    pub clock: i64,
    pub catalog: EngineCatalog,
}

/// How the reviewed table is identified in its description.
pub(crate) enum Identity {
    Postgres {
        database_oid: u32,
        relation_oid: u32,
    },
    /// SHA-256 of the table's definition.
    Fingerprint(String),
}

impl Plan {
    /// Plan `intent` against an inspected catalog: the recipe's draft and
    /// capacity, the resolved columns, the frozen seed and clock and, when
    /// the recipe is runnable, its description.
    pub(crate) fn build(
        catalog: EngineCatalog,
        intent: TableSeedIntent,
        target: TableSeedConnection,
        identity: Identity,
    ) -> Result<Plan, Failure> {
        let table = catalog.table();
        // Capacity needs the row count, so it is checked here as well as at
        // execution: a recipe that cannot fit fails at review.
        let draft = draft(&table, &intent).and_then(|draft| {
            capacity(&draft, intent.row_count, &intent)?;
            Ok(draft)
        });
        let columns = columns(&table, draft.as_ref().ok());
        let inserted = columns
            .iter()
            .filter(|c| c.action != TableSeedColumnAction::Default)
            .count();
        let issue = draft
            .err()
            .or_else(|| (inserted == 0).then_some(TableSeedError::InvalidRecipe));
        let seed_used = intent.seed.unwrap_or_else(|| {
            let bytes = uuid::Uuid::new_v4().into_bytes();
            u64::from_le_bytes(bytes[..8].try_into().unwrap())
        });
        let clock = chrono::Utc::now().timestamp();
        let description = if issue.is_none() {
            let (database_oid, relation_oid, table_fingerprint) = match identity {
                Identity::Postgres {
                    database_oid,
                    relation_oid,
                } => (database_oid, relation_oid, None),
                Identity::Fingerprint(fingerprint) => (0, 0, Some(fingerprint)),
            };
            let (recipe_summary, recipe_summary_truncated) = summary(&intent);
            let description = TableSeedDescription {
                endpoint: intent.endpoint.clone(),
                connection: target,
                database_oid,
                relation_oid,
                row_count: intent.row_count,
                seed_used,
                clock_epoch_seconds: clock,
                recipe_sha256: recipe_sha256(&intent)?,
                recipe_summary,
                recipe_summary_truncated,
                inserted_columns: inserted as u16,
                defaulted_columns: (columns.len() - inserted) as u16,
                generator_version: seed::SEED_GENERATOR_VERSION,
                table_fingerprint,
            };
            description.validate()?;
            Some(description)
        } else {
            None
        };
        Ok(Plan {
            description,
            intent,
            columns,
            issue,
            seed_used,
            clock,
            catalog,
        })
    }

    /// Validate actual owned capacities before publishing a fixed 4 MiB
    /// capture.
    pub(crate) fn checked_heap_bytes(&self) -> Option<usize> {
        use std::mem::size_of;
        let mut n = size_of::<Plan>().checked_add(self.intent.checked_heap_bytes()?)?;
        if let Some(description) = &self.description {
            n = n.checked_add(description.checked_heap_bytes()?)?;
        }
        n = n.checked_add(
            self.columns
                .capacity()
                .checked_mul(size_of::<TableSeedColumn>())?,
        )?;
        for c in &self.columns {
            n = n.checked_add(c.name.capacity())?;
            n = n.checked_add(c.data_type.capacity())?;
            if let TableSeedColumnAction::ForeignKey { schema, table } = &c.action {
                n = n.checked_add(schema.capacity())?;
                n = n.checked_add(table.capacity())?;
            }
        }
        n = n.checked_add(self.catalog.heap_bytes()?)?;
        (n <= MAX_TABLE_SEED_REVIEW_BYTES).then_some(n)
    }

    /// Sample values for `intent` against this inspected table, without a
    /// database: the same planning and capacity checks as review, then a few
    /// generated rows. The preview seed is the intent's, or a fixed one.
    pub(crate) fn preview(
        &self,
        intent: &TableSeedIntent,
    ) -> Result<TableSeedPreview, TableSeedError> {
        if intent.endpoint != self.intent.endpoint {
            return Err(TableSeedError::StaleReview);
        }
        let table = self.catalog.table();
        let draft = draft(&table, intent)?;
        capacity(&draft, intent.row_count, intent)?;
        // Prepare refuses a recipe that inserts nothing; so does preview.
        let inserted = columns(&table, Some(&draft))
            .iter()
            .filter(|c| c.action != TableSeedColumnAction::Default)
            .count();
        if inserted == 0 {
            return Err(TableSeedError::InvalidRecipe);
        }
        let seed = intent.seed.unwrap_or(PREVIEW_SEED);
        // The inspected clock stands in for the one review will freeze.
        let rows = seed::preview_rows(
            draft,
            &table.structure,
            &options(intent),
            self.clock,
            seed,
            TABLE_SEED_PREVIEW_ROWS.min(intent.row_count),
            TABLE_SEED_PREVIEW_BYTES,
        )
        .map_err(|_| TableSeedError::InvalidRecipe)?
        .ok_or(TableSeedError::Limit)?;
        Ok(TableSeedPreview::new(seed, intent.seed.is_some(), rows))
    }
}

/// SHA-256 of the exact recipe: columns, locales and realism.
fn recipe_sha256(intent: &TableSeedIntent) -> Result<String, TableSeedError> {
    use sha2::{Digest, Sha256};
    let encoded = serde_json::to_string(&(&intent.columns, &intent.locales, intent.realism))
        .map_err(|_| TableSeedError::Limit)?;
    Ok(hex(&Sha256::digest(encoded.as_bytes())))
}

/// SHA-256 of a table definition, as lowercase hex.
pub(crate) fn fingerprint(definition: &str) -> String {
    use sha2::{Digest, Sha256};
    hex(&Sha256::digest(definition.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(64), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[derive(Debug)]
pub(crate) struct Failure {
    pub error: TableSeedError,
    pub diagnostic: Option<Box<TableSeedDiagnostic>>,
}
impl From<TableSeedError> for Failure {
    fn from(error: TableSeedError) -> Self {
        Self {
            error,
            diagnostic: None,
        }
    }
}

impl Failure {
    /// A database error with its code, constraint and column, each bounded.
    pub(crate) fn database(
        code: Option<&str>,
        constraint: Option<&str>,
        column: Option<&str>,
    ) -> Failure {
        let bounded = |s: Option<&str>| {
            s.filter(|s| s.len() <= 63 && !s.contains('\0'))
                .map(str::to_owned)
        };
        let code = code
            .filter(|c| {
                (1..=8).contains(&c.len())
                    && c.bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            })
            .map(str::to_owned);
        let diagnostic = (code.is_some() || constraint.is_some() || column.is_some()).then(|| {
            Box::new(TableSeedDiagnostic {
                sqlstate: code,
                constraint: bounded(constraint),
                column: bounded(column),
                parent_schema: None,
                parent_table: None,
            })
        });
        Failure {
            error: TableSeedError::Database,
            diagnostic,
        }
    }

    /// A referenced parent with no rows to sample.
    pub(crate) fn empty_parent(fk: &crate::ForeignKeyInfo) -> Failure {
        Failure {
            error: TableSeedError::EmptyParent,
            diagnostic: Some(Box::new(TableSeedDiagnostic {
                sqlstate: None,
                constraint: Some(fk.name.clone()).filter(|n| n.len() <= 63),
                column: fk.columns.first().cloned(),
                parent_schema: Some(fk.referenced_schema.clone()).filter(|s| !s.is_empty()),
                parent_table: Some(fk.referenced_table.clone()),
            })),
        }
    }
}

/// A database error from sqlx, with its SQLSTATE or result code.
pub(crate) fn sqlx_failure(error: &sqlx::Error) -> Failure {
    match error.as_database_error() {
        Some(e) => Failure::database(e.code().as_deref(), e.constraint(), None),
        None => TableSeedError::Database.into(),
    }
}

pub(crate) struct Execution {
    pub outcome: TableSeedOutcome,
    pub failure: Option<Failure>,
}

impl Execution {
    /// Nothing was written: `began` says whether a transaction was opened
    /// (and is now rolled back).
    pub(crate) fn failed(began: bool, failure: Failure) -> Execution {
        Execution {
            outcome: if began {
                TableSeedOutcome::RolledBack
            } else {
                TableSeedOutcome::NotStarted
            },
            failure: Some(failure),
        }
    }
}

/// The overall budget of one seed's write transaction.
pub(crate) const EXECUTION_DEADLINE: Duration = Duration::from_secs(600);
/// The inspection budget.
pub(crate) const INSPECTION_DEADLINE: Duration = Duration::from_secs(30);

/// Commit fence shared by every engine: past the deadline or after a cancel
/// the transaction is rolled back; once COMMIT is sent, a failure or timeout
/// leaves the outcome unknown, because the server may have committed.
pub(crate) async fn commit_result(
    future: impl std::future::Future<Output = Result<(), Failure>>,
    control: &Control,
    rows: u64,
    deadline: Instant,
) -> Execution {
    if Instant::now() >= deadline {
        return Execution {
            outcome: TableSeedOutcome::RolledBack,
            failure: Some(TableSeedError::Timeout.into()),
        };
    }
    if !control.admit_commit() {
        return Execution {
            outcome: TableSeedOutcome::RolledBack,
            failure: Some(TableSeedError::Cancelled.into()),
        };
    }
    tokio::pin!(future);
    let deadline = deadline.min(Instant::now() + Duration::from_secs(10));
    let result = tokio::select! {biased;r=&mut future=>r,_=tokio::time::sleep_until(deadline)=>Err(TableSeedError::Timeout.into()),_=control.cancelled()=>match tokio::time::timeout_at(deadline.min(Instant::now()+Duration::from_secs(1)),&mut future).await{Ok(r)=>r,Err(_)=>Err(TableSeedError::Cancelled.into())}};
    match result {
        Ok(()) => Execution {
            outcome: TableSeedOutcome::Completed { rows },
            failure: None,
        },
        Err(e) => Execution {
            outcome: TableSeedOutcome::OutcomeUnknown,
            failure: Some(e),
        },
    }
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

pub(crate) fn specs(intent: &TableSeedIntent) -> Vec<crate::SeedColumnSpec> {
    intent
        .columns
        .iter()
        .map(|s| {
            let (mut skip, mut constant, mut values, mut generator) = (false, None, None, None);
            match &s.source {
                TableSeedSource::Default => skip = true,
                TableSeedSource::Constant { value } => constant = Some(value.clone()),
                TableSeedSource::Values { values: v } => values = Some(v.clone()),
                TableSeedSource::Auto { generator: g } => generator = g.map(|g| g.id().to_owned()),
            }
            crate::SeedColumnSpec {
                column: s.column.clone(),
                skip,
                constant,
                values,
                generator,
                min: None,
                max: None,
                min_length: s.length.and_then(|l| l.min).map(|v| v as usize),
                max_length: s.length.and_then(|l| l.max).map(|v| v as usize),
                null_rate: s.null_rate,
            }
        })
        .collect()
}

fn skipped(column: &str) -> crate::SeedColumnSpec {
    crate::SeedColumnSpec {
        column: column.to_owned(),
        skip: true,
        constant: None,
        values: None,
        generator: None,
        min: None,
        max: None,
        min_length: None,
        max_length: None,
        null_rate: None,
    }
}

pub(crate) fn draft(
    table: &SeedTable,
    intent: &TableSeedIntent,
) -> Result<seed::PlanDraft, TableSeedError> {
    for s in &intent.columns {
        let c = table
            .columns
            .iter()
            .find(|c| c.name == s.column)
            .ok_or(TableSeedError::InvalidRecipe)?;
        if (c.generated || c.identity)
            && !matches!(
                s.source,
                TableSeedSource::Default | TableSeedSource::Auto { generator: None }
            )
        {
            return Err(TableSeedError::InvalidRecipe);
        }
    }
    let mut specs = specs(intent);
    // An enum's declared labels are the only values it accepts, so an Auto
    // column draws from them instead of falling back to NULL.
    for (column, labels) in &table.enum_labels {
        match specs.iter_mut().find(|s| &s.column == column) {
            Some(spec)
                if !spec.skip
                    && spec.constant.is_none()
                    && spec.values.is_none()
                    && spec.generator.is_none() =>
            {
                spec.values = Some(labels.clone());
            }
            Some(_) => {}
            None => specs.push(crate::SeedColumnSpec {
                values: Some(labels.clone()),
                skip: false,
                ..skipped(column)
            }),
        }
    }
    for c in table.columns.iter().filter(|c| c.generated || c.identity) {
        match specs.iter_mut().find(|s| s.column == c.name) {
            Some(spec) => spec.skip = true,
            None => specs.push(skipped(&c.name)),
        }
    }
    seed::analyze_plan(table.dialect, &table.structure, &specs).map_err(|e| match e.kind {
        seed::PlanErrorKind::Length => TableSeedError::LengthRange,
        seed::PlanErrorKind::Unsupported => TableSeedError::UnsupportedColumn,
    })
}

/// Generation options carried by the intent.
pub(crate) fn options(intent: &TableSeedIntent) -> seed::GenOptions {
    seed::GenOptions {
        locales: intent.locales.clone(),
        realism: intent.realism,
    }
}

pub(crate) fn columns(table: &SeedTable, draft: Option<&seed::PlanDraft>) -> Vec<TableSeedColumn> {
    let text = seed::auto_takes_length(table.dialect, &table.structure);
    table
        .columns
        .iter()
        .enumerate()
        .map(|(index, c)| {
            let action = match draft
                .and_then(|d| d.columns().get(index))
                .map(|c| &c.source)
            {
                Some(ColumnSource::Skip) => TableSeedColumnAction::Default,
                Some(ColumnSource::Constant(Some(_))) => TableSeedColumnAction::Constant,
                Some(ColumnSource::Constant(None)) => TableSeedColumnAction::UnsupportedNull,
                Some(ColumnSource::ValueList(_)) => TableSeedColumnAction::Values,
                Some(ColumnSource::FkPool { pool, .. }) => {
                    let fk = &table.structure.foreign_keys[*pool];
                    TableSeedColumnAction::ForeignKey {
                        schema: fk.referenced_schema.clone(),
                        table: fk.referenced_table.clone(),
                    }
                }
                _ => TableSeedColumnAction::Auto,
            };
            let enumerated = table.enum_labels.iter().any(|(name, _)| name == &c.name);
            TableSeedColumn {
                name: c.name.clone(),
                data_type: c.data_type.clone(),
                nullable: c.nullable,
                has_default: c.has_default,
                generated: c.generated,
                identity: c.identity,
                action,
                auto_length: text.get(index).copied().unwrap_or(false) && !enumerated,
            }
        })
        .collect()
}

pub(crate) fn summary(intent: &TableSeedIntent) -> (String, bool) {
    use std::fmt::Write;
    let mut out = String::new();
    let mut truncated = false;
    for s in &intent.columns {
        let source = match &s.source {
            TableSeedSource::Default => "DEFAULT".into(),
            TableSeedSource::Auto { generator } => {
                generator.map(|g| g.id()).unwrap_or("Auto").into()
            }
            TableSeedSource::Constant { value } => format!(
                "constant {:?}{} ({} UTF-8 bytes)",
                value.chars().take(64).collect::<String>(),
                if value.chars().count() > 64 {
                    " [preview clipped]"
                } else {
                    ""
                },
                value.len()
            ),
            TableSeedSource::Values { values } => format!(
                "{} values; first {:?} [full list in live review]",
                values.len(),
                values[0].chars().take(64).collect::<String>()
            ),
        };
        let length = match s.length {
            Some(l) => format!(
                "; length {}..{}",
                l.min.map_or("auto".into(), |v| v.to_string()),
                l.max.map_or("auto".into(), |v| v.to_string())
            ),
            None => String::new(),
        };
        let line = format!(
            "{}: {}; NULL rate {:?}{length}\n",
            s.column, source, s.null_rate
        );
        if out.len() + line.len() > 8192 {
            truncated = true;
            break;
        }
        let _ = out.write_str(&line);
    }
    if out.is_empty() {
        out.push_str("All columns Auto; database-supplied columns omitted\n");
    }
    let locales = intent
        .locales
        .iter()
        .map(|l| l.id())
        .collect::<Vec<_>>()
        .join(", ");
    let realism = match intent.realism {
        TableSeedRealism::FictionalSafe => "fictional-safe",
        TableSeedRealism::Realistic => "realistic",
    };
    let line = format!("Locales {locales}; {realism} contact details\n");
    if out.len() + line.len() > 8192 {
        truncated = true;
    } else {
        out.push_str(&line);
    }
    (out, truncated)
}

/// Unique columns must have room for `row_count` distinct values: integer
/// digits for fixed-point numbers, characters for tagged text, numbers for
/// phones.
pub(crate) fn capacity(
    draft: &seed::PlanDraft,
    row_count: u32,
    intent: &TableSeedIntent,
) -> Result<(), TableSeedError> {
    seed::check_capacity(draft, row_count, &options(intent)).map_err(|e| match e {
        seed::CapacityError::Numeric => TableSeedError::IntegerRange,
        seed::CapacityError::Length => TableSeedError::LengthRange,
        seed::CapacityError::Phone => TableSeedError::PhoneRange,
    })
}

/// Rows per INSERT: bounded by `max_params` bind parameters, 8 MiB of
/// worst-case generated values (with a second copy for parameters) and
/// 1 MiB of SQL. `placeholder_bytes` is the SQL text of one value
/// placeholder per column.
pub(crate) fn batch_size(
    plan: &seed::SeedPlan,
    placeholder_bytes: &[usize],
    prefix_bytes: usize,
    max_params: usize,
) -> Result<u32, TableSeedError> {
    let mut row = 0usize;
    for c in &plan.columns {
        if matches!(c.source, ColumnSource::Skip) {
            continue;
        }
        let bytes = seed::worst_case_value_bytes(plan, c);
        row = row
            .checked_add(bytes.saturating_mul(4) + 128)
            .ok_or(TableSeedError::Limit)?;
    }
    if placeholder_bytes.is_empty() || row > 8 * 1024 * 1024 {
        return Err(TableSeedError::Limit);
    }
    let sql_row = placeholder_bytes
        .iter()
        .try_fold(0usize, |n, t| n.checked_add(t + 32))
        .ok_or(TableSeedError::Limit)?;
    let count = 500usize
        .min(max_params / placeholder_bytes.len())
        .min(8 * 1024 * 1024 / row.max(1))
        .min(
            (1024usize * 1024)
                .checked_sub(prefix_bytes)
                .ok_or(TableSeedError::Limit)?
                / sql_row.max(1),
        );
    if count == 0 {
        return Err(TableSeedError::Limit);
    }
    Ok(count as u32)
}

/// The run-wide write budget, shared by engine runners.
pub(crate) fn deadline() -> Instant {
    Instant::now() + EXECUTION_DEADLINE
}

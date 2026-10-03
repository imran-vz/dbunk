//! Bounded descriptions support recovery, never recreate write authority.
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of};
pub const MAX_TABLE_SEED_ACTIVE: usize = 4;
pub const MAX_TABLE_SEED_TERMINAL: usize = 32;
pub const MAX_TABLE_SEED_ROWS: u32 = 1_000_000;
pub const MAX_TABLE_SEED_COLUMNS: usize = 1600;
pub const MAX_TABLE_SEED_SPEC_BYTES: usize = 128 * 1024;
pub const MAX_TABLE_SEED_REVIEW_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_TABLE_SEED_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_TABLE_SEED_LIST_BYTES: usize = 1024 * 1024;
pub const MAX_TABLE_SEED_EXECUTION_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_TABLE_SEED_EXECUTION_POOL_BYTES: usize = 128 * 1024 * 1024;
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableSeedAttemptId(pub(super) uuid::Uuid);
impl TableSeedAttemptId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
    pub fn parse(text: &str) -> Result<Self, TableSeedError> {
        let id = uuid::Uuid::parse_str(text).map_err(|_| TableSeedError::InvalidRequest)?;
        if id.get_version_num() != 4 || id.to_string() != text {
            return Err(TableSeedError::InvalidRequest);
        }
        Ok(Self(id))
    }
}
impl Default for TableSeedAttemptId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Display for TableSeedAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl fmt::Debug for TableSeedAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Serialize for TableSeedAttemptId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for TableSeedAttemptId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedEndpoint {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}
impl TableSeedEndpoint {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let mut n = size_of::<Self>();
        for (s, max) in [
            (&self.connection_id, 128),
            (&self.schema, 63),
            (&self.table, 63),
        ] {
            if s.is_empty() || s.len() > max || s.capacity() > max * 2 || s.contains('\0') {
                return None;
            }
            n = n.checked_add(s.capacity())?;
        }
        Some(n)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedConnection {
    pub connection_name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub environment: String,
    pub safe_mode: String,
    pub read_only: bool,
}
impl TableSeedConnection {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.port == 0
            || !matches!(
                self.environment.as_str(),
                "Development" | "Test" | "Staging" | "Production"
            )
            || !matches!(
                self.safe_mode.as_str(),
                "Inherit" | "Disabled" | "Protected" | "Strict"
            )
        {
            return None;
        }
        let mut n = size_of::<Self>();
        for s in [
            &self.connection_name,
            &self.host,
            &self.database,
            &self.user,
            &self.environment,
            &self.safe_mode,
        ] {
            if s.len() > 256 || s.capacity() > 512 || s.contains('\0') {
                return None;
            }
            n = n.checked_add(s.capacity())?;
        }
        Some(n)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableSeedGenerator {
    Email,
    FirstName,
    LastName,
    FullName,
    UserName,
    Company,
    Url,
    Phone,
    City,
    Country,
    StreetAddress,
    Word,
    Sentence,
    Boolean,
    TinyInt,
    SmallInt,
    Integer,
    BigInt,
    Float,
    Decimal,
    Price,
    Uuid,
    Date,
    Time,
    Timestamp,
    Json,
}
impl TableSeedGenerator {
    pub fn id(self) -> &'static str {
        match self {
            Self::Email => "email",
            Self::FirstName => "firstName",
            Self::LastName => "lastName",
            Self::FullName => "fullName",
            Self::UserName => "userName",
            Self::Company => "company",
            Self::Url => "url",
            Self::Phone => "phone",
            Self::City => "city",
            Self::Country => "country",
            Self::StreetAddress => "streetAddress",
            Self::Word => "word",
            Self::Sentence => "sentence",
            Self::Boolean => "boolean",
            Self::TinyInt => "tinyInt",
            Self::SmallInt => "smallInt",
            Self::Integer => "integer",
            Self::BigInt => "bigInt",
            Self::Float => "float",
            Self::Decimal => "decimal",
            Self::Price => "price",
            Self::Uuid => "uuid",
            Self::Date => "date",
            Self::Time => "time",
            Self::Timestamp => "timestamp",
            Self::Json => "json",
        }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum TableSeedSource {
    Auto {
        generator: Option<TableSeedGenerator>,
    },
    Default,
    Constant {
        value: String,
    },
    Values {
        values: Vec<String>,
    },
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedColumnSpec {
    pub column: String,
    pub source: TableSeedSource,
    pub null_rate: Option<f64>,
}
#[derive(Clone)]
pub struct TableSeedIntent {
    pub endpoint: TableSeedEndpoint,
    pub row_count: u32,
    pub seed: Option<u64>,
    pub columns: Vec<TableSeedColumnSpec>,
}
impl TableSeedIntent {
    pub fn new(
        endpoint: TableSeedEndpoint,
        row_count: u32,
        seed: Option<u64>,
        columns: Vec<TableSeedColumnSpec>,
    ) -> Result<Self, TableSeedError> {
        let value = Self {
            endpoint,
            row_count,
            seed,
            columns,
        };
        value
            .checked_heap_bytes()
            .ok_or(TableSeedError::InvalidRequest)?;
        Ok(value)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !(1..=MAX_TABLE_SEED_ROWS).contains(&self.row_count)
            || self.columns.len() > MAX_TABLE_SEED_COLUMNS
        {
            return None;
        }
        let mut n = size_of::<Self>()
            .checked_add(self.endpoint.checked_heap_bytes()?)?
            .checked_add(
                self.columns
                    .capacity()
                    .checked_mul(size_of::<TableSeedColumnSpec>())?,
            )?;
        for (i, c) in self.columns.iter().enumerate() {
            if !name(&c.column)
                || self.columns[..i].iter().any(|p| p.column == c.column)
                || c.null_rate
                    .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
            {
                return None;
            }
            n = n.checked_add(c.column.capacity())?;
            let mut literal = |s: &String| -> Option<()> {
                if s.len() > 8192 || s.contains('\0') {
                    return None;
                }
                n = n.checked_add(s.capacity())?;
                Some(())
            };
            match &c.source {
                TableSeedSource::Constant { value } => literal(value)?,
                TableSeedSource::Values { values } => {
                    if values.is_empty() || values.len() > 1024 {
                        return None;
                    }
                    for value in values {
                        literal(value)?;
                    }
                    n = n.checked_add(values.capacity().checked_mul(size_of::<String>())?)?;
                }
                _ => {}
            }
        }
        if n > MAX_TABLE_SEED_SPEC_BYTES {
            return None;
        }
        super::bounds::encoded(&self.columns, MAX_TABLE_SEED_SPEC_BYTES)?;
        Some(n)
    }
}
impl fmt::Debug for TableSeedIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TableSeedIntent")
            .field("row_count", &self.row_count)
            .field("columns", &self.columns.len())
            .finish_non_exhaustive()
    }
}
pub(super) fn name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 63 && !s.contains('\0')
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "DescriptionFields")]
pub struct TableSeedDescription {
    pub endpoint: TableSeedEndpoint,
    pub connection: TableSeedConnection,
    pub database_oid: u32,
    pub relation_oid: u32,
    pub row_count: u32,
    pub seed_used: u64,
    pub clock_epoch_seconds: i64,
    pub recipe_sha256: String,
    pub recipe_summary: String,
    pub recipe_summary_truncated: bool,
    pub inserted_columns: u16,
    pub defaulted_columns: u16,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DescriptionFields {
    endpoint: TableSeedEndpoint,
    connection: TableSeedConnection,
    database_oid: u32,
    relation_oid: u32,
    row_count: u32,
    seed_used: u64,
    clock_epoch_seconds: i64,
    recipe_sha256: String,
    recipe_summary: String,
    recipe_summary_truncated: bool,
    inserted_columns: u16,
    defaulted_columns: u16,
}
impl TryFrom<DescriptionFields> for TableSeedDescription {
    type Error = TableSeedError;
    fn try_from(f: DescriptionFields) -> Result<Self, Self::Error> {
        let value = Self {
            endpoint: f.endpoint,
            connection: f.connection,
            database_oid: f.database_oid,
            relation_oid: f.relation_oid,
            row_count: f.row_count,
            seed_used: f.seed_used,
            clock_epoch_seconds: f.clock_epoch_seconds,
            recipe_sha256: f.recipe_sha256,
            recipe_summary: f.recipe_summary,
            recipe_summary_truncated: f.recipe_summary_truncated,
            inserted_columns: f.inserted_columns,
            defaulted_columns: f.defaulted_columns,
        };
        value.validate()?;
        Ok(value)
    }
}

impl TableSeedDescription {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.database_oid == 0
            || self.relation_oid == 0
            || !(1..=MAX_TABLE_SEED_ROWS).contains(&self.row_count)
            || self.recipe_sha256.len() != 64
            || !self.recipe_sha256.bytes().all(|c| c.is_ascii_hexdigit())
            || self.inserted_columns == 0
            || usize::from(self.inserted_columns) + usize::from(self.defaulted_columns)
                > MAX_TABLE_SEED_COLUMNS
        {
            return None;
        }
        if self.recipe_summary.len() > 8192 || self.recipe_summary.contains('\0') {
            return None;
        }
        let n = size_of::<Self>()
            .checked_add(self.recipe_summary.capacity())?
            .checked_add(self.endpoint.checked_heap_bytes()?)?
            .checked_add(self.connection.checked_heap_bytes()?)?
            .checked_add(self.recipe_sha256.capacity())?;
        super::bounds::encoded(self, MAX_TABLE_SEED_DESCRIPTION_BYTES)?;
        (n <= MAX_TABLE_SEED_DESCRIPTION_BYTES).then_some(n)
    }
    pub fn validate(&self) -> Result<(), TableSeedError> {
        self.checked_heap_bytes()
            .map(|_| ())
            .ok_or(TableSeedError::InvalidRequest)
    }
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub enum TableSeedColumnAction {
    Default,
    Constant,
    Values,
    ForeignKey { schema: String, table: String },
    Auto,
    UnsupportedNull,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TableSeedColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub has_default: bool,
    pub generated: bool,
    pub identity: bool,
    pub action: TableSeedColumnAction,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableSeedPhase {
    Preparing,
    NeedsRecipe,
    ReadyReview,
    AwaitingConfirmation,
    Running,
    Committing,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}
impl TableSeedPhase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableSeedOutcome {
    NotStarted,
    Pending,
    RolledBack,
    Completed { rows: u64 },
    OutcomeUnknown,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableSeedCleanup {
    Pending,
    Complete,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TableSeedError {
    InvalidRequest,
    InvalidRecipe,
    UnsupportedTarget,
    UnsupportedColumn,
    EmptyParent,
    IntegerRange,
    PolicyBlocked,
    Credentials,
    Database,
    StaleReview,
    TargetChanged,
    Missing,
    DuplicateAttempt,
    Busy,
    Closing,
    Cancelled,
    Timeout,
    Cleanup,
    Limit,
}
impl fmt::Display for TableSeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "Invalid or oversized seed request",
            Self::InvalidRecipe => "Seed recipe names an unavailable column or incompatible source",
            Self::UnsupportedTarget => "Seeding requires a supported PostgreSQL table",
            Self::UnsupportedColumn => {
                "No Auto generator for a required column; provide a constant or value list"
            }
            Self::EmptyParent => {
                "A referenced parent has no non-NULL tuples; seed the named parent first"
            }
            Self::IntegerRange => "Unique integer generation would exceed the column range",
            Self::PolicyBlocked => "Stored connection policy blocks seeding",
            Self::Credentials => "Connection credentials are unavailable",
            Self::Database => "PostgreSQL rejected the seed operation; inspect constraint details",
            Self::StaleReview => "Seed review or connection changed; prepare a new attempt",
            Self::TargetChanged => "Table or constraints changed since review",
            Self::Missing => "Seed attempt is unavailable",
            Self::DuplicateAttempt => "Attempt already exists; observe it instead of retrying",
            Self::Busy => "Seed work capacity or connection is occupied",
            Self::Closing => "Connection or native backend is closing",
            Self::Cancelled => "Seed cancellation requested",
            Self::Timeout => "Seed operation exceeded its deadline",
            Self::Cleanup => "Seed connection cleanup was not established",
            Self::Limit => "Seed recipe, metadata, sample or batch exceeds native bounds",
        })
    }
}
impl std::error::Error for TableSeedError {}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedDiagnostic {
    pub sqlstate: Option<String>,
    pub constraint: Option<String>,
    pub column: Option<String>,
    pub parent_schema: Option<String>,
    pub parent_table: Option<String>,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedReceipt {
    pub attempt_id: TableSeedAttemptId,
    pub description: TableSeedDescription,
    pub outcome: TableSeedOutcome,
    pub failure: Option<TableSeedError>,
    pub diagnostic: Option<TableSeedDiagnostic>,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct TableSeedObservation {
    pub attempt_id: TableSeedAttemptId,
    pub endpoint: TableSeedEndpoint,
    pub row_count: u32,
    pub seed_used: Option<u64>,
    pub phase: TableSeedPhase,
    pub outcome: TableSeedOutcome,
    pub cleanup: TableSeedCleanup,
    pub rows_generated: u64,
    pub issue: Option<TableSeedError>,
    pub failure: Option<TableSeedError>,
    pub diagnostic: Option<TableSeedDiagnostic>,
    pub receipt: Option<TableSeedReceipt>,
    pub change_revision: Option<u64>,
}
#[derive(Clone, Serialize)]
pub struct TableSeedList {
    pub jobs: Vec<TableSeedObservation>,
    pub change_revision: u64,
}

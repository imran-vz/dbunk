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
/// Longest text value a length range may request, in characters.
pub const MAX_TABLE_SEED_TEXT_CHARS: u32 = crate::seed::MAX_GENERATED_CHARS as u32;
/// Most locales one run may mix.
pub const MAX_TABLE_SEED_LOCALES: usize = 8;
/// Rows a preview generates.
pub const TABLE_SEED_PREVIEW_ROWS: u32 = 8;
/// Characters of each previewed value kept for display.
pub const TABLE_SEED_PREVIEW_CHARS: usize = 120;
/// Worst-case bytes a preview may generate before it is cut for display.
pub const TABLE_SEED_PREVIEW_BYTES: usize = 16 * 1024 * 1024;
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
    MiddleName,
    Paragraph,
    AddressLine2,
    Region,
    PostalCode,
    CountryCode,
    FullAddress,
}
impl TableSeedGenerator {
    /// Whether this generator builds text and so takes a length range.
    pub fn accepts_length(self) -> bool {
        crate::seed::parse_generator_id(self.id()).is_ok_and(|kind| kind.text_limits().is_some())
    }
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
            Self::MiddleName => "middleName",
            Self::Paragraph => "paragraph",
            Self::AddressLine2 => "addressLine2",
            Self::Region => "region",
            Self::PostalCode => "postalCode",
            Self::CountryCode => "countryCode",
            Self::FullAddress => "fullAddress",
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
/// Inclusive character-length range for an Auto text column. Either end
/// may be left to the column and generator; the backend intersects the
/// rest with both and builds values to fit (ADR-0014).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedLength {
    pub min: Option<u32>,
    pub max: Option<u32>,
}
impl TableSeedLength {
    fn valid(self) -> bool {
        let in_bounds =
            |v: Option<u32>| v.is_none_or(|v| (1..=MAX_TABLE_SEED_TEXT_CHARS).contains(&v));
        (self.min.is_some() || self.max.is_some())
            && in_bounds(self.min)
            && in_bounds(self.max)
            && match (self.min, self.max) {
                (Some(min), Some(max)) => min <= max,
                _ => true,
            }
    }
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TableSeedColumnSpec {
    pub column: String,
    pub source: TableSeedSource,
    pub null_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length: Option<TableSeedLength>,
}
/// A locale the built-in name and address data covers.
pub use crate::seed::Locale as TableSeedLocale;
/// Whether contact details may look like real, assigned ones.
pub use crate::seed::Realism as TableSeedRealism;
#[derive(Clone)]
pub struct TableSeedIntent {
    pub endpoint: TableSeedEndpoint,
    pub row_count: u32,
    pub seed: Option<u64>,
    pub columns: Vec<TableSeedColumnSpec>,
    /// Never empty; each generated row draws one.
    pub locales: Vec<TableSeedLocale>,
    pub realism: TableSeedRealism,
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
            locales: vec![TableSeedLocale::EnUs],
            realism: TableSeedRealism::default(),
        };
        value
            .checked_heap_bytes()
            .ok_or(TableSeedError::InvalidRequest)?;
        Ok(value)
    }
    /// Choose the locales rows are drawn from and whether contact details
    /// may look real.
    pub fn with_generation(
        mut self,
        locales: Vec<TableSeedLocale>,
        realism: TableSeedRealism,
    ) -> Result<Self, TableSeedError> {
        self.locales = locales;
        self.realism = realism;
        self.checked_heap_bytes()
            .ok_or(TableSeedError::InvalidRequest)?;
        Ok(self)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if !(1..=MAX_TABLE_SEED_ROWS).contains(&self.row_count)
            || self.columns.len() > MAX_TABLE_SEED_COLUMNS
            || self.locales.is_empty()
            || self.locales.len() > MAX_TABLE_SEED_LOCALES
            || self
                .locales
                .iter()
                .enumerate()
                .any(|(i, l)| self.locales[..i].contains(l))
        {
            return None;
        }
        let mut n = size_of::<Self>()
            .checked_add(
                self.locales
                    .capacity()
                    .checked_mul(size_of::<TableSeedLocale>())?,
            )?
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
                || c.length.is_some_and(|l| {
                    !l.valid() || !matches!(c.source, TableSeedSource::Auto { .. })
                })
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
    /// Generator version (data and algorithms) the rows were produced by;
    /// a seed reproduces its rows only on the same version. 0 predates
    /// versioning.
    pub generator_version: u32,
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
    #[serde(default)]
    generator_version: u32,
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
            generator_version: f.generator_version,
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
    /// Whether this column's Auto source is a text generator, which takes
    /// a length range.
    pub auto_length: bool,
}
/// One previewed cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TableSeedPreviewCell {
    /// `text` is the value cut to [`TABLE_SEED_PREVIEW_CHARS`] for display;
    /// `chars` is the full value's length.
    Value {
        text: String,
        chars: usize,
    },
    Null,
    /// The database's DEFAULT applies.
    Default,
    /// Sampled from the referenced parent table at run time.
    FromParent,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSeedPreviewColumn {
    pub name: String,
    pub cells: Vec<TableSeedPreviewCell>,
}
/// Sample rows of an edited recipe. Display only: it grants nothing and is
/// not what Apply runs, which is always the exact reviewed recipe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableSeedPreview {
    /// The seed the values were generated with.
    pub seed: u64,
    /// Whether that is the recipe's own seed. When the recipe leaves its
    /// seed to review, the preview uses a fixed one and the run's values
    /// will differ.
    pub seed_chosen: bool,
    pub columns: Vec<TableSeedPreviewColumn>,
}
impl TableSeedPreview {
    pub(crate) fn new(seed: u64, seed_chosen: bool, columns: crate::seed::PreviewColumns) -> Self {
        use crate::seed::PreviewCell;
        let columns = columns
            .into_iter()
            .map(|(name, cells)| TableSeedPreviewColumn {
                name,
                cells: cells
                    .into_iter()
                    .map(|cell| match cell {
                        PreviewCell::Value(value) => TableSeedPreviewCell::Value {
                            chars: value.chars().count(),
                            text: value.chars().take(TABLE_SEED_PREVIEW_CHARS).collect(),
                        },
                        PreviewCell::Null => TableSeedPreviewCell::Null,
                        PreviewCell::Default => TableSeedPreviewCell::Default,
                        PreviewCell::Parent => TableSeedPreviewCell::FromParent,
                    })
                    .collect(),
            })
            .collect();
        Self {
            seed,
            seed_chosen,
            columns,
        }
    }
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
    LengthRange,
    PhoneRange,
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
            Self::IntegerRange => "Unique value generation would exceed the column range",
            Self::LengthRange => {
                "A column's length range does not fit the column, its generator or the row count"
            }
            Self::PhoneRange => {
                "A unique phone column needs more numbers than the fictional-safe ranges hold"
            }
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

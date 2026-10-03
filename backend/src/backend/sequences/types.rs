use super::super::data::DataDocument;
use serde::Serialize;
use std::{fmt, mem::size_of, sync::Arc};

pub const MAX_SEQUENCE_REVIEW_BYTES: usize = 32 * 1024;
pub const MAX_SEQUENCE_RECEIPT_BYTES: usize = 32 * 1024;
pub const SEQUENCE_OPERATION_TIMEOUT_MS: u32 = 30_000;
pub const SEQUENCE_INSPECT_LIMIT: &str = "Inspection reads catalog metadata and SELECTs last_value/is_called from the sequence relation inside a READ ONLY transaction. It never calls nextval. Other sessions may advance the sequence at any time, so the observed value is a point-in-time reading, and per-session cached values are not visible.";
pub const SEQUENCE_IDENTITY_LIMIT: &str = "Advance and Set are bound to the observed sequence OID and run only if the database, schema and sequence OIDs, names and definition still match in the same statement. Restart resolves the quoted name; the identity and definition are rechecked before and after ALTER SEQUENCE in one transaction, but a concurrent schema rename between those checks is not fully locked.";
pub const SEQUENCE_EFFECT_LIMIT: &str = "nextval and setval are not transactional: once PostgreSQL executes them the change persists even if the reply is lost. ALTER SEQUENCE RESTART is transactional and commits with its own COMMIT. A lost reply after dispatch is reported as unknown and is never retried; inspect the sequence again before another action.";

/// Read-only identity for display. Constructing it never grants execution
/// authority; only a backend observation can mint a review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceTarget {
    pub(crate) database_oid: u32,
    pub(crate) database: String,
    pub(crate) namespace_oid: u32,
    pub(crate) schema: String,
    pub(crate) sequence_oid: u32,
    pub(crate) name: String,
}
impl SequenceTarget {
    pub fn database_oid(&self) -> u32 {
        self.database_oid
    }
    pub fn database(&self) -> &str {
        &self.database
    }
    pub fn namespace_oid(&self) -> u32 {
        self.namespace_oid
    }
    pub fn schema(&self) -> &str {
        &self.schema
    }
    pub fn sequence_oid(&self) -> u32 {
        self.sequence_oid
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn qualified(&self) -> String {
        format!(
            "{}.{}",
            crate::quote_double(&self.schema),
            crate::quote_double(&self.name)
        )
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.database.capacity() + self.schema.capacity() + self.name.capacity()
    }
    pub(crate) fn valid(&self) -> bool {
        self.database_oid != 0
            && self.namespace_oid != 0
            && self.sequence_oid != 0
            && [&self.database, &self.schema, &self.name]
                .into_iter()
                .all(|s| valid_name(s))
    }
}
pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 63 && !name.contains('\0')
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SequenceDataType {
    Smallint,
    Integer,
    Bigint,
}
impl SequenceDataType {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        match text {
            "smallint" => Some(Self::Smallint),
            "integer" => Some(Self::Integer),
            "bigint" => Some(Self::Bigint),
            _ => None,
        }
    }
    pub fn sql(self) -> &'static str {
        match self {
            Self::Smallint => "smallint",
            Self::Integer => "integer",
            Self::Bigint => "bigint",
        }
    }
    fn bounds(self) -> (i64, i64) {
        match self {
            Self::Smallint => (i16::MIN.into(), i16::MAX.into()),
            Self::Integer => (i32::MIN.into(), i32::MAX.into()),
            Self::Bigint => (i64::MIN, i64::MAX),
        }
    }
}

/// Stored sequence definition. Apply refuses if any of these values changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceDefinition {
    pub data_type: SequenceDataType,
    pub start: i64,
    pub increment: i64,
    pub min_value: i64,
    pub max_value: i64,
    pub cache: i64,
    pub cycle: bool,
}
impl SequenceDefinition {
    pub(crate) fn valid(&self) -> bool {
        let (low, high) = self.data_type.bounds();
        self.increment != 0
            && self.min_value < self.max_value
            && low <= self.min_value
            && self.max_value <= high
            && (self.min_value..=self.max_value).contains(&self.start)
            && self.cache >= 1
    }
    pub fn contains(&self, value: i64) -> bool {
        (self.min_value..=self.max_value).contains(&value)
    }
    /// Value the next nextval returns from (last_value, is_called), if it stays
    /// in range without cycling. `None` means the limit is reached.
    pub fn next_after(&self, last_value: i64, is_called: bool) -> Option<i64> {
        if !is_called {
            return Some(last_value).filter(|value| self.contains(*value));
        }
        last_value
            .checked_add(self.increment)
            .filter(|value| self.contains(*value))
    }
    pub fn describe_next(&self, last_value: i64, is_called: bool) -> String {
        match self.next_after(last_value, is_called) {
            Some(next) => format!("the next nextval is expected to return {next}"),
            None if self.cycle => format!(
                "the next nextval is expected to cycle to {}",
                if self.increment > 0 {
                    self.min_value
                } else {
                    self.max_value
                }
            ),
            None => "the next nextval is expected to fail at the sequence limit".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum SequenceValue {
    /// The role lacks SELECT on the sequence. The value is not read and nextval
    /// is never used as a substitute.
    NotReadable,
    Read {
        last_value: i64,
        is_called: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceObservation {
    pub target: SequenceTarget,
    pub definition: SequenceDefinition,
    pub value: SequenceValue,
    /// Quoted `schema.table.column` of an owning serial/identity column.
    pub owned_by: Option<String>,
    pub identity: bool,
}
impl SequenceObservation {
    pub(crate) fn valid(&self) -> bool {
        self.target.valid()
            && self.definition.valid()
            && self
                .owned_by
                .as_ref()
                .is_none_or(|owner| owner.len() <= 200)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.target.retained_bytes()
            + self.owned_by.as_ref().map_or(0, String::capacity)
    }
    pub fn text(&self) -> String {
        let d = &self.definition;
        let value = match self.value {
            SequenceValue::NotReadable => {
                "Current value: not readable (no SELECT privilege on this sequence)".to_owned()
            }
            SequenceValue::Read {
                last_value,
                is_called,
            } => format!(
                "last_value: {last_value}\nis_called: {is_called}\nObserved state: {}",
                d.describe_next(last_value, is_called)
            ),
        };
        format!(
            "Database: {:?} (OID {})\nSchema: {:?} (OID {})\nSequence: {:?} (OID {})\nData type: {}\nStart: {}\nIncrement: {}\nMinimum: {}\nMaximum: {}\nCache: {}\nCycle: {}\nOwned by: {}\n{value}\n",
            self.target.database,
            self.target.database_oid,
            self.target.schema,
            self.target.namespace_oid,
            self.target.name,
            self.target.sequence_oid,
            d.data_type.sql(),
            d.start,
            d.increment,
            d.min_value,
            d.max_value,
            d.cache,
            if d.cycle { "yes" } else { "no" },
            match (&self.owned_by, self.identity) {
                (Some(owner), true) => format!("{owner} (identity column)"),
                (Some(owner), false) => owner.clone(),
                (None, true) => "identity column (name exceeds display limit)".into(),
                (None, false) => "none".into(),
            }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum SequenceIntent {
    /// `nextval`: consumes exactly one value.
    Advance,
    /// `setval(seq, value, is_called)`.
    Set { value: i64, is_called: bool },
    /// `ALTER SEQUENCE ... RESTART [WITH value]`.
    Restart { with: Option<i64> },
}
impl SequenceIntent {
    pub(super) fn policy(self) -> crate::safety::policy::WriteIntent {
        use crate::postgres::sql_class::StatementClass;
        use crate::safety::policy::WriteIntent;
        match self {
            // Matches the SQL classifier for `SELECT nextval(...)`.
            Self::Advance => WriteIntent::Statement {
                classes: vec![StatementClass::Dml {
                    unbounded: false,
                    destructive: false,
                }],
            },
            // Rewinding a sequence can cause duplicate keys; Protected confirms.
            Self::Set { .. } => WriteIntent::Statement {
                classes: vec![StatementClass::Dml {
                    unbounded: false,
                    destructive: true,
                }],
            },
            // Same policy as the baseline typed ALTER SEQUENCE operation.
            Self::Restart { .. } => WriteIntent::Ddl,
        }
    }
    pub(super) fn command(self) -> &'static str {
        match self {
            Self::Advance => "advance_sequence",
            Self::Set { .. } => "set_sequence",
            Self::Restart { .. } => "restart_sequence",
        }
    }
    pub fn transactional(self) -> bool {
        matches!(self, Self::Restart { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequencePreview {
    /// Readable equivalent statement. The exact text sent is `sql`.
    pub summary: String,
    /// Exact statement text sent to PostgreSQL for the effect.
    pub sql: String,
    /// Bound parameter values in `$n` order, rendered for review.
    pub parameters: Vec<String>,
    pub effect: String,
    pub transactional: bool,
    pub operation_timeout_ms: u32,
    pub statement_timeout_ms: Option<u32>,
}
impl SequencePreview {
    pub fn identity_limit(&self) -> &'static str {
        SEQUENCE_IDENTITY_LIMIT
    }
    pub fn effect_limit(&self) -> &'static str {
        SEQUENCE_EFFECT_LIMIT
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.summary.capacity()
            + self.sql.capacity()
            + self.effect.capacity()
            + self.parameters.capacity() * size_of::<String>()
            + self.parameters.iter().map(String::capacity).sum::<usize>()
    }
    pub fn text(&self) -> String {
        format!(
            "Equivalent: {}\nExact SQL sent:\n{}\nParameters:\n{}\nEffect: {}\nTransactional: {}\nOperation deadline: {} ms\nStatement timeout: {}\n",
            self.summary,
            self.sql,
            self.parameters
                .iter()
                .enumerate()
                .map(|(index, value)| format!("  ${} = {value}\n", index + 1))
                .collect::<String>(),
            self.effect,
            if self.transactional {
                "yes (ALTER SEQUENCE commits explicitly)"
            } else {
                "no (the effect persists once executed)"
            },
            self.operation_timeout_ms,
            self.statement_timeout_ms
                .map_or_else(|| "inherited/server default".into(), |ms| format!(
                    "{ms} ms (configured)"
                ))
        )
    }
}

pub struct ObservedSequence {
    pub(super) document: DataDocument,
    pub(super) observation: SequenceObservation,
    pub(super) statement_timeout_ms: Option<u32>,
}
impl ObservedSequence {
    pub fn observation(&self) -> &SequenceObservation {
        &self.observation
    }
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>() + self.observation.retained_bytes() + document_bytes(&self.document)
    }
    /// Mint a single-use review for this exact observation. Inputs are checked
    /// against the observed definition before any dispatch is possible.
    pub fn review(&self, intent: SequenceIntent) -> Result<SequenceReview, SequenceError> {
        self.document
            .0
            .check_open()
            .map_err(|_| SequenceError::Unavailable)?;
        let review = SequenceReview {
            document: self.document.clone(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            intent,
            observation: self.observation.clone(),
            preview: super::preview(&self.observation, intent, self.statement_timeout_ms)?,
        };
        if review.retained_bytes() > MAX_SEQUENCE_REVIEW_BYTES {
            return Err(SequenceError::InvalidTarget);
        }
        Ok(review)
    }
}
pub struct SequenceReview {
    pub(super) document: DataDocument,
    pub(super) attempt_id: String,
    pub(super) intent: SequenceIntent,
    pub(super) observation: SequenceObservation,
    pub(super) preview: SequencePreview,
}
impl SequenceReview {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        Arc::ptr_eq(&self.document.0, &document.0)
    }
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }
    pub fn intent(&self) -> SequenceIntent {
        self.intent
    }
    pub fn observation(&self) -> &SequenceObservation {
        &self.observation
    }
    pub fn preview(&self) -> &SequencePreview {
        &self.preview
    }
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.observation.retained_bytes()
            + self.preview.retained_bytes()
            + document_bytes(&self.document)
    }
}
pub struct SequenceConfirmation {
    pub(super) review: SequenceReview,
}
impl SequenceConfirmation {
    pub fn belongs_to(&self, document: &DataDocument) -> bool {
        self.review.belongs_to(document)
    }
    pub fn review(&self) -> &SequenceReview {
        &self.review
    }
    pub fn retained_bytes(&self) -> usize {
        self.review.retained_bytes()
    }
}
pub enum SequenceSubmission {
    NeedsConfirmation(Box<SequenceConfirmation>),
    Finished(Box<SequenceReceipt>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SequenceReceipt {
    pub attempt_id: String,
    pub intent: SequenceIntent,
    pub target: SequenceTarget,
    pub preview: SequencePreview,
    pub outcome: SequenceOutcome,
    pub runtime_ms: u64,
}
impl SequenceReceipt {
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + self.attempt_id.capacity()
            + self.target.retained_bytes()
            + self.preview.retained_bytes()
            + self.outcome.heap_bytes()
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum SequenceOutcome {
    /// Nothing reached PostgreSQL as an effect statement.
    NotDispatched { reason: SequenceFailure },
    /// The exact observed identity or definition no longer matched; no effect.
    TargetChanged,
    /// Acknowledged. `returned` is the nextval/setval result; RESTART has none.
    Completed { returned: Option<i64> },
    /// PostgreSQL rejected the non-transactional call before it took effect.
    Rejected { reason: SequenceFailure },
    /// The RESTART transaction did not commit.
    RolledBack { reason: SequenceFailure },
    /// Dispatched, but the effect cannot be determined. Never retried.
    OutcomeUnknown { reason: SequenceFailure },
}
impl SequenceOutcome {
    fn heap_bytes(&self) -> usize {
        match self {
            Self::NotDispatched { reason }
            | Self::Rejected { reason }
            | Self::RolledBack { reason }
            | Self::OutcomeUnknown { reason } => match reason {
                SequenceFailure::Database { code } => code.as_ref().map_or(0, String::capacity),
                _ => 0,
            },
            _ => 0,
        }
    }
    pub fn unknown(&self) -> bool {
        matches!(self, Self::OutcomeUnknown { .. })
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum SequenceFailure {
    Cancelled,
    Timeout,
    Connection,
    Database { code: Option<String> },
}
impl fmt::Display for SequenceFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Sequence action interrupted"),
            Self::Timeout => f.write_str("Sequence action deadline reached"),
            Self::Connection => f.write_str("Sequence action connection failed"),
            Self::Database { code } => write!(
                f,
                "PostgreSQL sequence error{}",
                code.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default()
            ),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SequenceError {
    ForeignDocument,
    Unavailable,
    NotFound,
    InvalidTarget,
    /// Set/Restart value outside the observed MINVALUE..MAXVALUE range.
    OutOfRange,
    Storage,
    Busy,
    PolicyBlocked,
    OutcomeUnavailable,
}
impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ForeignDocument => "Sequence review belongs to another workspace",
            Self::Unavailable => "Sequence or document is unavailable",
            Self::NotFound => "Sequence not found; refresh Objects and select it again",
            Self::InvalidTarget => {
                "Sequence action requires a bounded, consistent sequence identity"
            }
            Self::OutOfRange => "Value is outside the observed sequence MINVALUE..MAXVALUE range",
            Self::Storage => "Stored connection metadata is unavailable",
            Self::Busy => "A schema or maintenance operation is still settling on this connection",
            Self::PolicyBlocked => "Stored read-only policy blocks sequence changes",
            Self::OutcomeUnavailable => {
                "Sequence outcome unavailable; inspect the sequence before another attempt"
            }
        })
    }
}
impl std::error::Error for SequenceError {}
fn document_bytes(document: &DataDocument) -> usize {
    std::mem::size_of_val(&*document.0)
        + document.0.window.capacity()
        + document.0.tab.capacity()
        + document.0.connection.capacity()
        + document.0.manager_tab.capacity()
}

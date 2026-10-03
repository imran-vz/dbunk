use serde::{de, Deserialize, Deserializer, Serialize};
use std::{fmt, mem::size_of};

pub const MAX_SCHEMA_NAME_BYTES: usize = 63;
pub const MAX_SCHEMA_COMMENT_BYTES: usize = 4096;
pub const MAX_SCHEMA_PREVIEW_BYTES: usize = 16 * 1024;
pub const MAX_SCHEMA_STATEMENTS: usize = 2;

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchemaIntent {
    name: String,
    comment: Option<String>,
}
impl CreateSchemaIntent {
    pub fn new(name: String, comment: Option<String>) -> Result<Self, CreateSchemaIntentError> {
        if name.trim().is_empty() || name.len() > MAX_SCHEMA_NAME_BYTES || name.contains('\0') {
            return Err(CreateSchemaIntentError::Name);
        }
        if comment
            .as_ref()
            .is_some_and(|text| text.len() > MAX_SCHEMA_COMMENT_BYTES || text.contains('\0'))
        {
            return Err(CreateSchemaIntentError::Comment);
        }
        // Discard excess caller capacity so valid short text cannot retain an
        // arbitrarily large backing allocation in a review or recovery record.
        Ok(Self {
            name: name.into_boxed_str().into_string(),
            comment: comment.map(|text| text.into_boxed_str().into_string()),
        })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn comment(&self) -> Option<&str> {
        self.comment.as_deref()
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.name.capacity() > MAX_SCHEMA_NAME_BYTES
            || self
                .comment
                .as_ref()
                .is_some_and(|text| text.capacity() > MAX_SCHEMA_COMMENT_BYTES)
        {
            return None;
        }
        size_of::<Self>()
            .checked_add(self.name.capacity())?
            .checked_add(self.comment.as_ref().map_or(0, String::capacity))
    }
}
impl<'de> Deserialize<'de> for CreateSchemaIntent {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Fields {
            name: String,
            comment: Option<String>,
        }
        let fields = Fields::deserialize(deserializer)?;
        Self::new(fields.name, fields.comment).map_err(de::Error::custom)
    }
}
impl fmt::Debug for CreateSchemaIntent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateSchemaIntent")
            .field("name_bytes", &self.name.len())
            .field("comment_bytes", &self.comment.as_ref().map(String::len))
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateSchemaIntentError {
    Name,
    Comment,
}
impl fmt::Display for CreateSchemaIntentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Name => {
                "Schema name must contain 1 to 63 UTF-8 bytes, be nonblank and contain no NUL"
            }
            Self::Comment => "Schema comment must contain at most 4096 UTF-8 bytes and no NUL",
        })
    }
}
impl std::error::Error for CreateSchemaIntentError {}

#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CreateSchemaAttemptId(String);
impl CreateSchemaAttemptId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl Default for CreateSchemaAttemptId {
    fn default() -> Self {
        Self::new()
    }
}
impl fmt::Debug for CreateSchemaAttemptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CreateSchemaAttemptId(<opaque>)")
    }
}
impl<'de> Deserialize<'de> for CreateSchemaAttemptId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let id = uuid::Uuid::parse_str(&text)
            .map_err(|_| de::Error::custom("Invalid schema attempt identity"))?;
        if id.get_version_num() != 4
            || id.get_variant() != uuid::Variant::RFC4122
            || id.to_string() != text
        {
            return Err(de::Error::custom("Invalid schema attempt identity"));
        }
        Ok(Self(text.into_boxed_str().into_string()))
    }
}

/// Bounded generated SQL for display. Execution regenerates it from the intent.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchemaPreview {
    pub statements: Vec<CreateSchemaStatement>,
}
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchemaStatement {
    pub sql: String,
    pub summary: String,
}
impl fmt::Debug for CreateSchemaPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CreateSchemaPreview")
            .field("statements", &self.statements.len())
            .finish()
    }
}
impl fmt::Debug for CreateSchemaStatement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CreateSchemaStatement(<redacted>)")
    }
}
impl CreateSchemaPreview {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.statements.is_empty()
            || self.statements.len() > MAX_SCHEMA_STATEMENTS
            || self.statements.capacity() > MAX_SCHEMA_STATEMENTS
        {
            return None;
        }
        let mut bytes = size_of::<Self>().checked_add(
            self.statements
                .capacity()
                .checked_mul(size_of::<CreateSchemaStatement>())?,
        )?;
        for statement in &self.statements {
            bytes = bytes
                .checked_add(statement.sql.capacity())?
                .checked_add(statement.summary.capacity())?;
        }
        (bytes <= MAX_SCHEMA_PREVIEW_BYTES
            && super::encoded_bytes(self) <= MAX_SCHEMA_PREVIEW_BYTES)
            .then_some(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSchemaReceipt {
    pub attempt_id: CreateSchemaAttemptId,
    pub connection_id: String,
    pub intent: CreateSchemaIntent,
    pub outcome: CreateSchemaOutcome,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CreateSchemaOutcome {
    Applied { statements: u8, runtime_ms: u64 },
    NotApplied { reason: CreateSchemaFailure },
    OutcomeUnknown { reason: CreateSchemaFailure },
}
/// Never carries server messages, SQL text, comments or connection secrets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CreateSchemaFailure {
    Cancelled,
    Timeout,
    Connection,
    Database { code: Option<String> },
}
impl fmt::Display for CreateSchemaFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Schema write cancelled"),
            Self::Timeout => f.write_str("Schema write exceeded its operation deadline"),
            Self::Connection => f.write_str("Schema connection failed"),
            Self::Database { code } => write!(
                f,
                "Schema operation failed{}",
                code.as_ref()
                    .map(|code| format!(" [{code}]"))
                    .unwrap_or_default()
            ),
        }
    }
}
#[derive(Debug)]
pub enum CreateSchemaError {
    Cancelled,
    ForeignDocument,
    OutcomeUnavailable,
    Unavailable,
    Storage,
    Busy,
    PolicyBlocked,
    InvalidPreview,
}
impl fmt::Display for CreateSchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Cancelled => "Schema request cancelled before dispatch",
            Self::ForeignDocument => "Schema review belongs to another document",
            Self::OutcomeUnavailable => {
                "Schema outcome unavailable; inspect the database before retrying"
            }
            Self::Unavailable => "Schema review is unavailable or its document has retired",
            Self::Storage => "Stored connection metadata is unavailable",
            Self::Busy => "A schema write is already active on this connection",
            Self::PolicyBlocked => "This connection is read-only",
            Self::InvalidPreview => "Generated schema preview exceeds its bounds or is invalid",
        })
    }
}
impl std::error::Error for CreateSchemaError {}

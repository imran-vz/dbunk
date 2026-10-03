//! Saved export recipes are local configuration, never captured rows, file paths
//! or execution authority. Rerunning requires a fresh target capture and picker.
use super::Backend;
use serde::{Deserialize, Serialize};
use std::{fmt, mem::size_of, sync::Arc};

mod storage;
#[cfg(test)]
mod tests;

pub const MAX_CONFIGURATIONS: usize = 128;
pub const MAX_CONFIGURATION_BYTES: usize = 256 * 1024;
pub const MAX_CONFIGURATION_HEAP_BYTES: usize = 512 * 1024;
pub const MAX_NULL_TOKEN_BYTES: usize = 8192;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportFormat {
    #[default]
    Csv,
    Json,
    Sql,
    Html,
    Markdown,
    Txt,
    Xlsx,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportEncoding {
    #[default]
    Utf8,
    Utf16Le,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportCompression {
    #[default]
    None,
    Gzip,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportOptions {
    pub format: ExportFormat,
    pub encoding: ExportEncoding,
    pub compression: ExportCompression,
    pub null_token: String,
}
impl ExportOptions {
    pub fn validate(&self) -> Result<(), ExportConfigurationError> {
        if self.null_token.len() > MAX_NULL_TOKEN_BYTES
            || self.null_token.capacity() > MAX_CONFIGURATION_HEAP_BYTES
        {
            Err(ExportConfigurationError::TooLarge)
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportTarget {
    pub connection_id: String,
    pub schema: String,
    pub table: String,
}
impl ExportTarget {
    pub fn validate(&self) -> Result<(), ExportConfigurationError> {
        for (value, limit) in [
            (&self.connection_id, 256),
            (&self.schema, 63),
            (&self.table, 63),
        ] {
            if value.is_empty() || value.len() > limit || value.contains('\0') {
                return Err(ExportConfigurationError::Invalid);
            }
        }
        if self.heap_bytes() > MAX_CONFIGURATION_HEAP_BYTES {
            return Err(ExportConfigurationError::TooLarge);
        }
        Ok(())
    }
    fn heap_bytes(&self) -> usize {
        self.connection_id
            .capacity()
            .saturating_add(self.schema.capacity())
            .saturating_add(self.table.capacity())
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedExportConfiguration {
    #[serde(with = "uuid_string")]
    pub id: uuid::Uuid,
    pub target: ExportTarget,
    pub options: ExportOptions,
    pub created_at: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExportConfigurationsRevision(Option<uuid::Uuid>);

#[derive(Clone, Debug)]
pub struct ExportConfigurationsCapture {
    record: Arc<storage::Record>,
    encoded: usize,
}
impl ExportConfigurationsCapture {
    pub fn revision(&self) -> ExportConfigurationsRevision {
        ExportConfigurationsRevision(
            self.record
                .revision
                .as_ref()
                .map(|text| uuid::Uuid::parse_str(text).expect("validated stored revision")),
        )
    }
    pub fn records(&self) -> &[SavedExportConfiguration] {
        &self.record.configurations
    }
    /// Storage ordering is the acknowledged save ordering, not wall-clock order.
    pub fn latest(&self, target: &ExportTarget) -> Option<&SavedExportConfiguration> {
        self.records()
            .iter()
            .find(|record| record.target == *target)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.record.checked_heap_bytes()
    }
    pub fn encoded_bytes(&self) -> usize {
        self.encoded
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportConfigurationError {
    Invalid,
    TooLarge,
    Full,
    Corrupt,
    UnsupportedVersion,
    StaleRevision,
    Storage,
    Unavailable,
}
impl fmt::Display for ExportConfigurationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "Invalid export configuration target or options",
            Self::TooLarge => {
                "Export configurations exceed their byte limit; existing records preserved"
            }
            Self::Full => "128 export configurations are already saved; existing records preserved",
            Self::Corrupt => "Saved export configurations are corrupt; existing records preserved",
            Self::UnsupportedVersion => {
                "Saved export configurations use an unsupported version; existing records preserved"
            }
            Self::StaleRevision => "Export configurations changed; reload before saving",
            Self::Storage => "Could not read or save export configurations",
            Self::Unavailable => "Export configuration storage is unavailable",
        })
    }
}
impl std::error::Error for ExportConfigurationError {}

// Keep the pinned uuid feature graph unchanged. Stored UUIDs use canonical text.
mod uuid_string {
    use serde::{Deserialize, Serializer};
    pub(super) fn serialize<S: Serializer>(
        id: &uuid::Uuid,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(id)
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<uuid::Uuid, D::Error> {
        let text = String::deserialize(d)?;
        let id = uuid::Uuid::parse_str(&text).map_err(serde::de::Error::custom)?;
        if id.to_string() != text {
            return Err(serde::de::Error::custom("noncanonical UUID"));
        }
        Ok(id)
    }
}

impl Backend {
    /// Local profile storage only, available while disconnected. Deleted target
    /// connections remain visible recipes and confer no permission to execute.
    pub async fn load_export_configurations(
        &self,
    ) -> Result<ExportConfigurationsCapture, ExportConfigurationError> {
        self.call(|state| async move { Ok(storage::load(&state.pool).await) })
            .await
            .map_err(|_| ExportConfigurationError::Unavailable)?
    }
    /// Creates a new recipe, matching baseline Save. The exact committed capture
    /// is returned; concurrent stale saves never overwrite an intervening save.
    pub async fn save_export_configuration(
        &self,
        target: ExportTarget,
        options: ExportOptions,
        expected: ExportConfigurationsRevision,
    ) -> Result<ExportConfigurationsCapture, ExportConfigurationError> {
        target.validate()?;
        options.validate()?;
        self.call(move |state| async move {
            Ok(storage::save(&state.pool, target, options, expected).await)
        })
        .await
        .map_err(|_| ExportConfigurationError::Unavailable)?
    }
}

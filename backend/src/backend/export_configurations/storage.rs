use super::*;
use serde::de::{SeqAccess, Visitor};
use sqlx::{SqliteConnection, SqlitePool};
use std::io::{self, Write};

pub(super) const KEY: &str = "native.export-configurations.v1";
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    version: u32,
    pub revision: Option<String>,
    #[serde(deserialize_with = "configurations")]
    pub configurations: Vec<SavedExportConfiguration>,
}
fn configurations<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<SavedExportConfiguration>, D::Error> {
    struct Records;
    impl<'de> Visitor<'de> for Records {
        type Value = Vec<SavedExportConfiguration>;
        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("at most 128 export configurations")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut values = Vec::new();
            while let Some(value) = seq.next_element()? {
                if values.len() == MAX_CONFIGURATIONS {
                    return Err(serde::de::Error::custom("too many export configurations"));
                }
                values.push(value);
            }
            Ok(values)
        }
    }
    deserializer.deserialize_seq(Records)
}
impl Record {
    fn empty() -> Self {
        Self {
            version: 1,
            revision: None,
            configurations: Vec::new(),
        }
    }
    pub(super) fn checked_heap_bytes(&self) -> Option<usize> {
        let mut bytes = size_of::<Self>()
            .checked_add(size_of::<ExportConfigurationsCapture>())?
            .checked_add(2 * size_of::<usize>())?
            .checked_add(self.revision.as_ref().map_or(0, String::capacity))?
            .checked_add(
                self.configurations
                    .capacity()
                    .checked_mul(size_of::<SavedExportConfiguration>())?,
            )?;
        if self.configurations.len() > MAX_CONFIGURATIONS {
            return None;
        }
        for record in &self.configurations {
            record.target.validate().ok()?;
            record.options.validate().ok()?;
            bytes = bytes
                .checked_add(record.target.heap_bytes())?
                .checked_add(record.options.null_token.capacity())?
                .checked_add(record.created_at.capacity())?;
        }
        (bytes <= MAX_CONFIGURATION_HEAP_BYTES).then_some(bytes)
    }
    fn validate(&self) -> Result<(), ExportConfigurationError> {
        if self.version != 1 {
            return Err(ExportConfigurationError::UnsupportedVersion);
        }
        let valid_revision = self.revision.as_ref().is_some_and(|text| {
            uuid::Uuid::parse_str(text).is_ok_and(|id| {
                id.get_version() == Some(uuid::Version::Random) && id.to_string() == *text
            })
        });
        if !valid_revision {
            return Err(ExportConfigurationError::Corrupt);
        }
        self.checked_heap_bytes()
            .ok_or(ExportConfigurationError::TooLarge)?;
        let mut ids = std::collections::HashSet::new();
        for record in &self.configurations {
            if record.id.get_version() != Some(uuid::Version::Random)
                || !ids.insert(record.id)
                || record.created_at.len() > 64
                || chrono::DateTime::parse_from_rfc3339(&record.created_at).is_err()
            {
                return Err(ExportConfigurationError::Corrupt);
            }
        }
        Ok(())
    }
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_CONFIGURATION_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("export configuration limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(record: &Record) -> Result<String, ExportConfigurationError> {
    let mut bytes = Bounded(Vec::new());
    serde_json::to_writer(&mut bytes, record).map_err(|_| ExportConfigurationError::TooLarge)?;
    String::from_utf8(bytes.0).map_err(|_| ExportConfigurationError::Invalid)
}
async fn read(conn: &mut SqliteConnection) -> Result<(Record, usize), ExportConfigurationError> {
    let row: Option<(i64, Option<String>)> = sqlx::query_as("SELECT length(CAST(value AS BLOB)), CASE WHEN length(CAST(value AS BLOB)) <= ? THEN value END FROM ui_state WHERE key=?")
        .bind(MAX_CONFIGURATION_BYTES as i64).bind(KEY).fetch_optional(conn).await.map_err(|_| ExportConfigurationError::Storage)?;
    let Some((length, encoded)) = row else {
        return Ok((Record::empty(), 0));
    };
    if length > MAX_CONFIGURATION_BYTES as i64 {
        return Err(ExportConfigurationError::TooLarge);
    }
    let encoded = encoded.ok_or(ExportConfigurationError::Corrupt)?;
    #[derive(Deserialize)]
    struct Version {
        version: u32,
    }
    let version: Version =
        serde_json::from_str(&encoded).map_err(|_| ExportConfigurationError::Corrupt)?;
    if version.version != 1 {
        return Err(ExportConfigurationError::UnsupportedVersion);
    }
    let record: Record =
        serde_json::from_str(&encoded).map_err(|_| ExportConfigurationError::Corrupt)?;
    record.validate()?;
    Ok((record, encoded.len()))
}
pub(super) async fn load(
    pool: &SqlitePool,
) -> Result<ExportConfigurationsCapture, ExportConfigurationError> {
    let mut conn = pool
        .acquire()
        .await
        .map_err(|_| ExportConfigurationError::Storage)?;
    let (record, encoded) = read(&mut conn).await?;
    Ok(ExportConfigurationsCapture {
        record: Arc::new(record),
        encoded,
    })
}
pub(super) async fn save(
    pool: &SqlitePool,
    target: ExportTarget,
    options: ExportOptions,
    expected: ExportConfigurationsRevision,
) -> Result<ExportConfigurationsCapture, ExportConfigurationError> {
    target.validate()?;
    options.validate()?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| ExportConfigurationError::Storage)?;
    let (mut record, _) = read(&mut tx).await?;
    if record.revision.as_deref() != expected.0.as_ref().map(|id| id.to_string()).as_deref() {
        return Err(ExportConfigurationError::StaleRevision);
    }
    if record.configurations.len() == MAX_CONFIGURATIONS {
        return Err(ExportConfigurationError::Full);
    }
    record.revision = Some(uuid::Uuid::new_v4().to_string());
    record.configurations.insert(
        0,
        SavedExportConfiguration {
            id: uuid::Uuid::new_v4(),
            target,
            options,
            created_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    record.validate()?;
    let encoded = encode(&record)?;
    let acknowledgement = ExportConfigurationsCapture {
        encoded: encoded.len(),
        record: Arc::new(record),
    };
    sqlx::query("INSERT INTO ui_state(key,value,updated_at) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at")
        .bind(KEY).bind(encoded).bind(acknowledgement.record.revision.as_ref().unwrap()).execute(&mut *tx).await.map_err(|_| ExportConfigurationError::Storage)?;
    tx.commit()
        .await
        .map_err(|_| ExportConfigurationError::Storage)?;
    Ok(acknowledgement)
}

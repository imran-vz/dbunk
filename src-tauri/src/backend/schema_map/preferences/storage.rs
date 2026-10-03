use super::*;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use std::io::{self, Write};
const PREFIX: &str = "native.schema-map.preferences.v1:";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u32,
    revision: String,
    connection_id: String,
    scope: SchemaMapPreferenceScope,
    #[serde(deserialize_with = "present_value")]
    value: Option<SchemaMapPreferences>,
}
fn present_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SchemaMapPreferences>, D::Error> {
    Option::<SchemaMapPreferences>::deserialize(deserializer)
}

#[derive(Deserialize)]
struct Version {
    version: u32,
}

pub(super) fn key(
    connection: &str,
    scope: &SchemaMapPreferenceScope,
) -> Result<String, SchemaMapPreferencesError> {
    if connection.is_empty()
        || connection.len() > 256
        || connection.contains('\0')
        || scope.checked_heap_bytes().is_none()
    {
        return Err(SchemaMapPreferencesError::Invalid);
    }
    let mut key = PREFIX.to_owned();
    key.push_str(
        &serde_json::to_string(&(connection, scope))
            .map_err(|_| SchemaMapPreferencesError::Invalid)?,
    );
    if key.len() > 4096 {
        return Err(SchemaMapPreferencesError::TooLarge);
    }
    Ok(key)
}
fn decode(
    encoded: &str,
    connection: &str,
    scope: &SchemaMapPreferenceScope,
) -> Result<Record, SchemaMapPreferencesError> {
    if encoded.len() > MAX_MAP_PREFERENCES_BYTES {
        return Err(SchemaMapPreferencesError::TooLarge);
    }
    let version: Version =
        serde_json::from_str(encoded).map_err(|_| SchemaMapPreferencesError::Corrupt)?;
    if version.version != 1 {
        return Err(SchemaMapPreferencesError::UnsupportedVersion);
    }
    let record: Record =
        serde_json::from_str(encoded).map_err(|_| SchemaMapPreferencesError::Corrupt)?;
    if record.connection_id != connection
        || record.scope != *scope
        || record.scope.checked_heap_bytes().is_none()
        || record
            .value
            .as_ref()
            .is_some_and(|v| v.checked_heap_bytes().is_none())
    {
        return Err(SchemaMapPreferencesError::Corrupt);
    }
    generation(&record.revision)?;
    Ok(record)
}
fn generation(value: &str) -> Result<uuid::Uuid, SchemaMapPreferencesError> {
    let id = uuid::Uuid::parse_str(value).map_err(|_| SchemaMapPreferencesError::Corrupt)?;
    if id.get_version() != Some(uuid::Version::Random) || id.to_string() != value {
        return Err(SchemaMapPreferencesError::Corrupt);
    }
    Ok(id)
}
struct Bounded(Vec<u8>);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_MAP_PREFERENCES_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::other("map preference limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn encode(record: &Record) -> Result<String, SchemaMapPreferencesError> {
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, record).map_err(|_| SchemaMapPreferencesError::TooLarge)?;
    String::from_utf8(writer.0).map_err(|_| SchemaMapPreferencesError::Invalid)
}
async fn read(
    conn: &mut SqliteConnection,
    key: &str,
    connection: &str,
    scope: &SchemaMapPreferenceScope,
) -> Result<Option<Record>, SchemaMapPreferencesError> {
    // A single SQL statement measures bytes before materializing JSON in Rust.
    // Corrupt/future/oversized records are never silently treated as absence.
    let row: Option<(i64,Option<String>)> = sqlx::query_as("SELECT length(CAST(value AS BLOB)),CASE WHEN length(CAST(value AS BLOB)) <= ? THEN value END FROM ui_state WHERE key = ?")
        .bind(MAX_MAP_PREFERENCES_BYTES as i64).bind(key).fetch_optional(conn).await.map_err(|_| SchemaMapPreferencesError::Storage)?;
    row.map(|(size, encoded)| {
        if size > MAX_MAP_PREFERENCES_BYTES as i64 {
            return Err(SchemaMapPreferencesError::TooLarge);
        }
        decode(
            &encoded.ok_or(SchemaMapPreferencesError::Corrupt)?,
            connection,
            scope,
        )
    })
    .transpose()
}
fn capture(
    connection: &str,
    scope: SchemaMapPreferenceScope,
    key: String,
    record: Option<Record>,
) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
    let (generation, value) = match record {
        None => (None, None),
        Some(record) => (Some(generation(&record.revision)?), record.value),
    };
    let capture = SchemaMapPreferencesCapture {
        connection_id: connection.to_owned(),
        scope,
        revision: SchemaMapPreferencesRevision { key, generation },
        value,
    };
    capture
        .checked_heap_bytes()
        .ok_or(SchemaMapPreferencesError::TooLarge)?;
    Ok(capture)
}
pub(super) async fn load(
    pool: &SqlitePool,
    connection: &str,
    scope: SchemaMapPreferenceScope,
) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
    let key = key(connection, &scope)?;
    let mut conn = pool
        .acquire()
        .await
        .map_err(|_| SchemaMapPreferencesError::Storage)?;
    let record = read(&mut conn, &key, connection, &scope).await?;
    capture(connection, scope, key, record)
}
pub(super) async fn save(
    pool: &SqlitePool,
    connection: &str,
    scope: SchemaMapPreferenceScope,
    expected: SchemaMapPreferencesRevision,
    value: Option<SchemaMapPreferences>,
) -> Result<SchemaMapPreferencesCapture, SchemaMapPreferencesError> {
    let key = key(connection, &scope)?;
    if key != expected.key {
        return Err(SchemaMapPreferencesError::StaleRevision);
    }
    if value
        .as_ref()
        .is_some_and(|v| v.checked_heap_bytes().is_none())
    {
        return Err(SchemaMapPreferencesError::Invalid);
    }
    let record = Record {
        version: 1,
        revision: uuid::Uuid::new_v4().to_string(),
        connection_id: connection.to_owned(),
        scope: scope.clone(),
        value,
    };
    let encoded = encode(&record)?;
    // Construct/validate the exact return value before the commit. An admission
    // failure can never turn a committed write into a missing acknowledgement.
    let acknowledged = capture(connection, scope, key.clone(), Some(record))?;
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(|_| SchemaMapPreferencesError::Storage)?;
    let current = read(&mut tx, &key, connection, &acknowledged.scope).await?;
    let current_generation = current
        .as_ref()
        .map(|record| generation(&record.revision))
        .transpose()?;
    if current_generation != expected.generation {
        return Err(SchemaMapPreferencesError::StaleRevision);
    }
    sqlx::query("INSERT INTO ui_state(key,value,updated_at) VALUES (?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,updated_at=excluded.updated_at")
        .bind(&key).bind(encoded).bind(acknowledged.revision.generation.unwrap().to_string())
        .execute(&mut *tx).await.map_err(|_| SchemaMapPreferencesError::Storage)?;
    tx.commit()
        .await
        .map_err(|_| SchemaMapPreferencesError::Storage)?;
    Ok(acknowledged)
}

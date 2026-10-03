use super::*;
use bounds::{own, own_cell};

enum SectionError {
    Restricted,
    Failed(CatalogError),
}
impl From<CatalogError> for SectionError {
    fn from(error: CatalogError) -> Self {
        Self::Failed(error)
    }
}
fn sql_error(error: tokio_postgres::Error) -> SectionError {
    classify_error(error.code().map(|code| code.code()))
}
fn classify_error(code: Option<&str>) -> SectionError {
    if code == Some("42501") {
        SectionError::Restricted
    } else {
        SectionError::Failed(CatalogError::Database)
    }
}
async fn section<T>(
    client: &Client,
    result: Result<T, SectionError>,
) -> Result<ServerSection<T>, CatalogError> {
    let result = match result {
        Ok(value) => ServerSection::Loaded(value),
        Err(SectionError::Restricted) => {
            client
                .batch_execute("ROLLBACK TO SAVEPOINT server_section")
                .await
                .map_err(|_| CatalogError::Database)?;
            ServerSection::Restricted
        }
        Err(SectionError::Failed(error)) => return Err(error),
    };
    client
        .batch_execute("RELEASE SAVEPOINT server_section")
        .await
        .map_err(|_| CatalogError::Database)?;
    Ok(result)
}
async fn savepoint(client: &Client) -> Result<(), CatalogError> {
    client
        .batch_execute("SAVEPOINT server_section")
        .await
        .map_err(|_| CatalogError::Database)
}
async fn present(client: &Client, catalog: &'static str) -> Result<bool, CatalogError> {
    let row = client
        .query_one(
            "SELECT pg_catalog.to_regclass($1)::oid IS NOT NULL AS present",
            &[&catalog],
        )
        .await
        .map_err(|_| CatalogError::Database)?;
    get(&row, "present")
}

pub(super) async fn load(
    client: &Client,
    configured: Option<u32>,
) -> Result<ServerDetailsSnapshot, CatalogError> {
    let collected_start = chrono::Utc::now().to_rfc3339();
    let statement_timeout_ms = inspection_timeout(configured);
    begin_snapshot(client, Some(statement_timeout_ms)).await?;
    let identity_row = client
        .query_one(queries::IDENTITY, &[])
        .await
        .map_err(|_| CatalogError::Database)?;
    let database = required(&identity_row, "database")?;
    let reader_pid = get(&identity_row, "pid")?;
    let reader = ReaderContext {
        current_user: cell(&identity_row, "current_user")?,
        session_user: cell(&identity_row, "session_user")?,
        search_path: cell(&identity_row, "search_path")?,
        statement_timeout_ms,
        lock_timeout_ms: 2000,
    };
    savepoint(client).await?;
    let facts_row = section(
        client,
        client
            .query_one(queries::FACTS, &[])
            .await
            .map_err(sql_error),
    )
    .await?;
    let facts = match &facts_row {
        ServerSection::Loaded(row) => ServerSection::Loaded(ServerFacts {
            server_version: cell(row, "server_version")?,
            encoding: cell(row, "encoding")?,
            locale: cell(row, "locale")?,
            timezone: cell(row, "timezone")?,
        }),
        ServerSection::Restricted => ServerSection::Restricted,
        ServerSection::Unavailable => return Err(CatalogError::InvalidResponse),
    };
    let facts_heap = match &facts {
        ServerSection::Loaded(value) => bounds::facts(value),
        _ => Some(0),
    }
    .ok_or(CatalogError::InvalidResponse)?;
    // Fixed timestamp/struct room is reserved before copying any header text.
    let header_heap = size_of::<ServerDetailsSnapshot>()
        + 256
        + database.len()
        + bounds::reader(&reader).ok_or(CatalogError::InvalidResponse)?
        + facts_heap;
    if header_heap > HEADER_BYTES || reader_pid <= 0 {
        return Err(CatalogError::ServerDetailsLimit);
    }
    bounds::encoded(
        &bounds::Header {
            database: &database,
            reader_pid,
            collected_start: &collected_start,
            collected_end: "0000-00-00T00:00:00.000000000+00:00",
            reader: &reader,
            facts: &facts,
        },
        HEADER_BYTES - 128,
    )
    .ok_or(CatalogError::ServerDetailsLimit)?;
    let mut snapshot = ServerDetailsSnapshot {
        database: own(database)?,
        reader_pid,
        collected_start,
        collected_end: String::new(),
        reader: ReaderContext {
            current_user: own_cell(reader.current_user)?,
            session_user: own_cell(reader.session_user)?,
            search_path: own_cell(reader.search_path)?,
            statement_timeout_ms,
            lock_timeout_ms: 2000,
        },
        facts: match facts {
            ServerSection::Loaded(f) => ServerSection::Loaded(ServerFacts {
                server_version: own_cell(f.server_version)?,
                encoding: own_cell(f.encoding)?,
                locale: own_cell(f.locale)?,
                timezone: own_cell(f.timezone)?,
            }),
            ServerSection::Restricted => ServerSection::Restricted,
            ServerSection::Unavailable => unreachable!(),
        },
        settings: ServerSection::Unavailable,
        extensions: ServerSection::Unavailable,
    };
    if present(client, "pg_catalog.pg_settings").await? {
        savepoint(client).await?;
        snapshot.settings = section(client, settings(client).await).await?;
    }
    if present(client, "pg_catalog.pg_extension").await? {
        savepoint(client).await?;
        snapshot.extensions = section(client, extensions(client).await).await?;
    }
    client
        .batch_execute("COMMIT")
        .await
        .map_err(|_| CatalogError::Database)?;
    snapshot.collected_end = chrono::Utc::now().to_rfc3339();
    snapshot
        .checked_heap_bytes()
        .ok_or(CatalogError::ServerDetailsLimit)?;
    Ok(snapshot)
}

pub(super) fn inspection_timeout(configured: Option<u32>) -> u32 {
    configured
        .filter(|ms| *ms > 0)
        .unwrap_or(10_000)
        .min(10_000)
}

fn get<'a, T: tokio_postgres::types::FromSql<'a>>(
    row: &'a Row,
    field: &str,
) -> Result<T, CatalogError> {
    row.try_get(field)
        .map_err(|_| CatalogError::InvalidResponse)
}
fn raw<'a>(row: &'a Row, field: &str) -> Result<(Option<&'a str>, Option<i64>), CatalogError> {
    // Field names are static in this module, never SQL or user input.
    Ok((get(row, field)?, get(row, &format!("{field}_bytes"))?))
}
fn required<'a>(row: &'a Row, field: &str) -> Result<&'a str, CatalogError> {
    let (value, bytes) = raw(row, field)?;
    if bytes.is_some_and(|bytes| bytes > MAX_SERVER_IDENTITY_BYTES as i64) {
        return Err(CatalogError::ServerDetailsLimit);
    }
    match (value, bytes) {
        (Some(value), Some(bytes))
            if bytes >= 0
                && value.len() == bytes as usize
                && bounds::identity(&value).is_some() =>
        {
            Ok(value)
        }
        _ => Err(CatalogError::InvalidResponse),
    }
}
fn cell<'a>(row: &'a Row, field: &str) -> Result<ServerText<&'a str>, CatalogError> {
    let (value, bytes) = raw(row, field)?;
    decode_cell(value, bytes)
}
pub(super) fn decode_cell(
    value: Option<&str>,
    bytes: Option<i64>,
) -> Result<ServerText<&str>, CatalogError> {
    match (value, bytes) {
        (None, None) => Ok(ServerText::Null),
        (None, Some(bytes)) if bytes > MAX_SERVER_TEXT_BYTES as i64 => Ok(ServerText::Omitted {
            bytes: bytes as u64,
        }),
        (Some(value), Some(bytes))
            if bytes >= 0
                && bytes <= MAX_SERVER_TEXT_BYTES as i64
                && value.len() == bytes as usize =>
        {
            Ok(ServerText::Value(value))
        }
        _ => Err(CatalogError::InvalidResponse),
    }
}

async fn settings(client: &Client) -> Result<ServerRows<ServerSetting>, SectionError> {
    let (mut budget, mut captured) =
        bounds::RowBudget::new::<ServerSetting>(MAX_SERVER_SETTINGS, SETTINGS_BYTES)?;
    let mut limit = None;
    let cap = MAX_SERVER_SETTINGS as i64 + 1;
    let rows = client
        .query_raw(queries::SETTINGS, [&cap as &(dyn ToSql + Sync)])
        .await
        .map_err(sql_error)?;
    tokio::pin!(rows);
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
        if captured.len() == MAX_SERVER_SETTINGS {
            limit = Some(ServerLimit::RowLimit);
            break;
        }
        let name = required(&row, "name")?;
        if captured.iter().any(|row| row.name == name) {
            return Err(CatalogError::InvalidResponse.into());
        }
        let value = ServerSetting {
            name,
            category: required(&row, "category")?,
            source: required(&row, "source")?,
            setting: cell(&row, "setting")?,
            unit: cell(&row, "unit")?,
            short_desc: cell(&row, "short_desc")?,
            boot_val: cell(&row, "boot_val")?,
            reset_val: cell(&row, "reset_val")?,
            inspection_override: matches!(name, "statement_timeout" | "lock_timeout"),
        };
        let heap = bounds::setting(&value).ok_or(CatalogError::InvalidResponse)?;
        if !budget.admit(&value, heap) {
            limit = Some(ServerLimit::ByteLimit);
            break;
        }
        captured.push(ServerSetting {
            name: own(value.name)?,
            category: own(value.category)?,
            source: own(value.source)?,
            setting: own_cell(value.setting)?,
            unit: own_cell(value.unit)?,
            short_desc: own_cell(value.short_desc)?,
            boot_val: own_cell(value.boot_val)?,
            reset_val: own_cell(value.reset_val)?,
            inspection_override: value.inspection_override,
        });
    }
    Ok(ServerRows {
        rows: captured,
        limit,
    })
}

async fn extensions(client: &Client) -> Result<ServerRows<ServerExtension>, SectionError> {
    let (mut budget, mut captured) =
        bounds::RowBudget::new::<ServerExtension>(MAX_SERVER_EXTENSIONS, EXTENSIONS_BYTES)?;
    let mut limit = None;
    let cap = MAX_SERVER_EXTENSIONS as i64 + 1;
    let rows = client
        .query_raw(queries::EXTENSIONS, [&cap as &(dyn ToSql + Sync)])
        .await
        .map_err(sql_error)?;
    tokio::pin!(rows);
    while let Some(row) = rows.try_next().await.map_err(sql_error)? {
        if captured.len() == MAX_SERVER_EXTENSIONS {
            limit = Some(ServerLimit::RowLimit);
            break;
        }
        let value = ServerExtension {
            name: required(&row, "name")?,
            schema: required(&row, "schema")?,
            version: cell(&row, "version")?,
            description: cell(&row, "description")?,
        };
        if captured.iter().any(|row| row.name == value.name) {
            return Err(CatalogError::InvalidResponse.into());
        }
        let heap = bounds::extension(&value).ok_or(CatalogError::InvalidResponse)?;
        if !budget.admit(&value, heap) {
            limit = Some(ServerLimit::ByteLimit);
            break;
        }
        captured.push(ServerExtension {
            name: own(value.name)?,
            schema: own(value.schema)?,
            version: own_cell(value.version)?,
            description: own_cell(value.description)?,
        });
    }
    Ok(ServerRows {
        rows: captured,
        limit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn permission_is_the_only_sql_error_recovered_as_a_section_state() {
        assert!(matches!(
            classify_error(Some("42501")),
            SectionError::Restricted
        ));
        for code in [
            None,
            Some("42P01"),
            Some("57014"),
            Some("08006"),
            Some("XX000"),
        ] {
            assert!(matches!(
                classify_error(code),
                SectionError::Failed(CatalogError::Database)
            ));
        }
    }
}

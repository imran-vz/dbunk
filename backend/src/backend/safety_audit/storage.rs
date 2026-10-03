//! Native-only bounded SQLite reader. Existing Tauri audit reads/writes unchanged.
use super::super::Inner;
use super::{types::*, *};
use futures_util::TryStreamExt;
use serde::{
    de::{self, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use sqlx::{Row, SqlitePool};
use std::{fmt, mem::size_of, sync::Weak};

// Fixed reserve covers page keys, connection, next cursor and comma separators.
const ENVELOPE_RESERVE: usize = 4096;

pub(super) async fn load(
    pool: &SqlitePool,
    owner: Weak<Inner>,
    connection_id: String,
    cursor: Option<SafetyAuditCursor>,
) -> Result<SafetyAuditPage, SafetyAuditError> {
    let mut transaction = pool.begin().await.map_err(|_| SafetyAuditError::Storage)?;
    let watermark = match &cursor {
        Some(cursor) => cursor.watermark,
        None => sqlx::query_scalar::<_, Option<i64>>(
            "SELECT max(id) FROM safety_overrides WHERE connection_id=?",
        )
        .bind(&connection_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|_| SafetyAuditError::Storage)?
        .unwrap_or(0),
    };
    if watermark < 0 {
        return Err(SafetyAuditError::Corrupt);
    }
    // CASE guards materialization on the SQLite side, including the cap+1 probe.
    // BLOB length measures UTF-8 bytes, not SQLite character count. Wrong types
    // remain corrupt even when SQLite could coerce them into a printable string.
    let query = format!("SELECT id,
      CASE WHEN typeof(command)='text' AND length(CAST(command AS BLOB))<={MAX_AUDIT_COMMAND_BYTES} THEN command END AS command,
      CASE WHEN typeof(occurred_at)='text' AND length(CAST(occurred_at AS BLOB))<={MAX_AUDIT_TIMESTAMP_BYTES} THEN occurred_at END AS occurred_at,
      CASE WHEN typeof(classes)='text' AND length(CAST(classes AS BLOB))<={MAX_CLASSES_JSON_BYTES} THEN classes END AS classes,
      (length(CAST(command AS BLOB))>{MAX_AUDIT_COMMAND_BYTES} OR length(CAST(occurred_at AS BLOB))>{MAX_AUDIT_TIMESTAMP_BYTES} OR length(CAST(classes AS BLOB))>{MAX_CLASSES_JSON_BYTES}) AS oversized
      FROM safety_overrides WHERE connection_id=? AND id<=? AND (? IS NULL OR occurred_at<? OR (occurred_at=? AND id<?)) ORDER BY occurred_at DESC,id DESC LIMIT {}", MAX_AUDIT_PAGE_ROWS+1);
    let mut stream = sqlx::query(&query)
        .bind(&connection_id)
        .bind(watermark)
        .bind(cursor.as_ref().map(|cursor| cursor.occurred_at.as_str()))
        .bind(cursor.as_ref().map(|cursor| cursor.occurred_at.as_str()))
        .bind(cursor.as_ref().map(|cursor| cursor.occurred_at.as_str()))
        .bind(cursor.as_ref().map(|cursor| cursor.id))
        .fetch(&mut *transaction);
    let mut rows = Vec::with_capacity(MAX_AUDIT_PAGE_ROWS);
    if rows.capacity() != MAX_AUDIT_PAGE_ROWS {
        return Err(SafetyAuditError::TooLarge);
    }
    let mut heap = size_of::<SafetyAuditPage>()
        + connection_id.capacity()
        + rows.capacity() * size_of::<SafetyAuditRow>()
        + MAX_AUDIT_CURSOR_BYTES;
    let mut encoded = ENVELOPE_RESERVE;
    let mut limit = None;
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|_| SafetyAuditError::Storage)?
    {
        if rows.len() == MAX_AUDIT_PAGE_ROWS {
            limit = Some(SafetyAuditLimit::RowLimit);
            break;
        }
        if row
            .try_get::<bool, _>("oversized")
            .map_err(|_| SafetyAuditError::Corrupt)?
        {
            return Err(SafetyAuditError::TooLarge);
        }
        let id: i64 = row.try_get("id").map_err(|_| SafetyAuditError::Corrupt)?;
        let command = row
            .try_get::<Option<&str>, _>("command")
            .map_err(|_| SafetyAuditError::Corrupt)?
            .ok_or(SafetyAuditError::Corrupt)?;
        let occurred_at = row
            .try_get::<Option<&str>, _>("occurred_at")
            .map_err(|_| SafetyAuditError::Corrupt)?
            .ok_or(SafetyAuditError::Corrupt)?;
        let raw_classes = row
            .try_get::<Option<&str>, _>("classes")
            .map_err(|_| SafetyAuditError::Corrupt)?
            .ok_or(SafetyAuditError::Corrupt)?;
        if id <= 0 || !command_valid(command) || !timestamp_valid(occurred_at) {
            return Err(SafetyAuditError::Corrupt);
        }
        let classes = parse_classes(raw_classes)?;
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Borrowed<'a> {
            id: i64,
            command: &'a str,
            occurred_at: &'a str,
            classes: &'a [SafetyAuditClass],
        }
        let row_encoded = count_encoded(
            &Borrowed {
                id,
                command,
                occurred_at,
                classes: &classes,
            },
            MAX_AUDIT_PAGE_BYTES,
        )
        .ok_or(SafetyAuditError::TooLarge)?;
        let row_heap =
            command.len() + occurred_at.len() + classes.capacity() * size_of::<SafetyAuditClass>();
        if heap + row_heap > MAX_AUDIT_PAGE_BYTES
            || encoded + row_encoded + 1 > MAX_AUDIT_PAGE_BYTES
        {
            if rows.is_empty() {
                return Err(SafetyAuditError::TooLarge);
            }
            limit = Some(SafetyAuditLimit::ByteLimit);
            break;
        }
        heap += row_heap;
        encoded += row_encoded + 1;
        // No large persisted field was cloned before both budgets admitted it.
        rows.push(SafetyAuditRow {
            id,
            command: command.into(),
            classes,
            occurred_at: occurred_at.into(),
        });
    }
    drop(stream);
    transaction
        .commit()
        .await
        .map_err(|_| SafetyAuditError::Storage)?;
    let next_cursor = match limit {
        Some(_) => {
            let last = rows.last().ok_or(SafetyAuditError::Corrupt)?;
            Some(SafetyAuditCursor {
                owner,
                connection_id: connection_id.clone(),
                occurred_at: last.occurred_at.clone(),
                id: last.id,
                watermark,
            })
        }
        None => None,
    };
    let page = SafetyAuditPage {
        connection_id,
        rows,
        next_cursor,
        limit,
    };
    page.checked_heap_bytes()
        .ok_or(SafetyAuditError::TooLarge)?;
    Ok(page)
}

fn parse_classes(text: &str) -> Result<Vec<SafetyAuditClass>, SafetyAuditError> {
    struct Classes(Vec<SafetyAuditClass>);
    impl<'de> Deserialize<'de> for Classes {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Bounded;
            impl<'de> Visitor<'de> for Bounded {
                type Value = Classes;
                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("bounded array of known statement class labels")
                }
                fn visit_seq<A: SeqAccess<'de>>(
                    self,
                    mut sequence: A,
                ) -> Result<Classes, A::Error> {
                    let mut classes = Vec::with_capacity(MAX_AUDIT_CLASSES);
                    if classes.capacity() != MAX_AUDIT_CLASSES {
                        return Err(de::Error::custom("class allocation bound"));
                    }
                    while let Some(class) = sequence.next_element::<SafetyAuditClass>()? {
                        if classes.len() == MAX_AUDIT_CLASSES {
                            return Err(de::Error::custom("too many audit classes"));
                        }
                        classes.push(class);
                    }
                    Ok(Classes(classes))
                }
            }
            deserializer.deserialize_seq(Bounded)
        }
    }
    if text.len() > MAX_CLASSES_JSON_BYTES {
        return Err(SafetyAuditError::TooLarge);
    }
    serde_json::from_str::<Classes>(text)
        .map(|classes| classes.0)
        .map_err(|_| SafetyAuditError::Corrupt)
}

//! Shared limits for multi-query descriptions. Component rows are streamed and
//! charged before decoding; SQL assembly refuses before extending its buffer.
use super::*;
use serde::de::DeserializeOwned;

pub(super) const MAX_COMPONENTS: usize = MAX_DESCRIPTION_COMPONENTS;
#[derive(Default)]
pub(super) struct Budget {
    rows: usize,
    bytes: usize,
}
impl Budget {
    pub(super) fn admit(&mut self, payload: &str) -> Result<(), CatalogError> {
        if self.rows == MAX_COMPONENTS
            || payload.len() > MAX_DESCRIPTION_BYTES.saturating_sub(self.bytes)
        {
            return Err(CatalogError::DescriptionLimit);
        }
        self.rows += 1;
        self.bytes += payload.len();
        Ok(())
    }
}

/// `source` and guard names are internal constants; values use bound parameters.
/// LIMIT is inside the wrapper so metadata work cannot return an unbounded set.
pub(super) async fn rows<T: DeserializeOwned>(
    client: &Client,
    source: &str,
    params: &[&(dyn ToSql + Sync)],
    metadata: &[&str],
    expressions: &[&str],
    budget: &mut Budget,
) -> Result<Vec<T>, CatalogError> {
    let guards = metadata
        .iter()
        .map(|name| (name, MAX_TEXT_BYTES))
        .chain(
            expressions
                .iter()
                .map(|name| (name, MAX_DESCRIPTION_TEXT_BYTES)),
        )
        .map(|(name, cap)| {
            format!("coalesce(octet_length(convert_to(doc->>'{name}', 'UTF8')), 0) > {cap}")
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    let guards = if guards.is_empty() { "false" } else { &guards };
    let remaining = MAX_COMPONENTS - budget.rows + 1;
    let sql = format!("WITH source AS ({source} LIMIT {remaining}), encoded AS (SELECT to_jsonb(source) AS doc FROM source), checked AS (SELECT doc::text AS payload, ({guards}) AS oversized FROM encoded) SELECT CASE WHEN NOT oversized AND octet_length(convert_to(payload, 'UTF8')) <= {MAX_DESCRIPTION_BYTES} THEN payload END AS payload, (oversized OR octet_length(convert_to(payload, 'UTF8')) > {MAX_DESCRIPTION_BYTES}) AS oversized FROM checked");
    let stream = client
        .query_raw(&sql, params.iter().copied())
        .await
        .map_err(|_| CatalogError::Database)?;
    tokio::pin!(stream);
    let mut result = Vec::new();
    while let Some(row) = stream
        .try_next()
        .await
        .map_err(|_| CatalogError::Database)?
    {
        if row
            .try_get::<_, bool>("oversized")
            .map_err(|_| CatalogError::InvalidResponse)?
        {
            return Err(CatalogError::DescriptionLimit);
        }
        let payload: &str = row
            .try_get("payload")
            .map_err(|_| CatalogError::InvalidResponse)?;
        budget.admit(payload)?;
        result.push(serde_json::from_str(payload).map_err(|_| CatalogError::InvalidResponse)?);
    }
    Ok(result)
}
pub(super) fn one<T>(mut rows: Vec<T>) -> Result<T, CatalogError> {
    match rows.len() {
        0 => Err(CatalogError::ObjectNotFound),
        1 => Ok(rows.remove(0)),
        _ => Err(CatalogError::InvalidResponse),
    }
}

#[derive(Default)]
pub(super) struct Sql(String);
impl Sql {
    pub(super) fn push(&mut self, text: &str) -> Result<(), CatalogError> {
        if text.len() > MAX_DESCRIPTION_TEXT_BYTES.saturating_sub(self.0.len()) {
            return Err(CatalogError::DescriptionLimit);
        }
        self.0.push_str(text);
        Ok(())
    }
    pub(super) fn ident(&mut self, value: &str) -> Result<(), CatalogError> {
        check_metadata(Some(value))?;
        self.push("\"")?;
        self.escaped(value, '"')?;
        self.push("\"")
    }
    pub(super) fn literal(&mut self, value: &str) -> Result<(), CatalogError> {
        check_metadata(Some(value))?;
        self.push("E'")?;
        for ch in value.chars() {
            if matches!(ch, '\\' | '\'') {
                self.push(ch.encode_utf8(&mut [0; 4]))?;
            }
            self.push(ch.encode_utf8(&mut [0; 4]))?;
        }
        self.push("'")
    }
    fn escaped(&mut self, value: &str, escape: char) -> Result<(), CatalogError> {
        for ch in value.chars() {
            if ch == escape {
                self.push(ch.encode_utf8(&mut [0; 4]))?;
            }
            self.push(ch.encode_utf8(&mut [0; 4]))?;
        }
        Ok(())
    }
    pub(super) fn qualified(&mut self, schema: &str, name: &str) -> Result<(), CatalogError> {
        self.ident(schema)?;
        self.push(".")?;
        self.ident(name)
    }
    pub(super) fn target(&mut self, reference: &PgObjectRef) -> Result<(), CatalogError> {
        self.qualified(
            reference
                .schema
                .as_deref()
                .ok_or(CatalogError::InvalidReference)?,
            &reference.name,
        )
    }
    pub(super) fn finish(self) -> String {
        self.0
    }
}

pub(super) fn finish(
    reference: PgObjectRef,
    owner: Option<String>,
    comment: Option<String>,
    definition: Sql,
    facts: PgObjectFacts,
) -> Result<PgObjectDescription, CatalogError> {
    check_metadata(owner.as_deref())?;
    check_metadata(comment.as_deref())?;
    let description = PgObjectDescription {
        reference,
        owner,
        comment,
        definition_sql: Some(definition.finish()),
        facts,
    };
    if json_size(&description)? > MAX_DESCRIPTION_BYTES {
        return Err(CatalogError::DescriptionLimit);
    }
    Ok(description)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_count_and_byte_refusal_happen_before_decoding_or_retention() {
        let mut count = Budget::default();
        for _ in 0..MAX_COMPONENTS {
            count.admit("{}").unwrap();
        }
        assert_eq!(count.admit("{}"), Err(CatalogError::DescriptionLimit));
        let mut bytes = Budget::default();
        bytes.admit(&"x".repeat(MAX_DESCRIPTION_BYTES)).unwrap();
        assert_eq!(bytes.admit("x"), Err(CatalogError::DescriptionLimit));
    }
    #[test]
    fn sql_assembly_refuses_late_expansion_without_extending_buffer() {
        let mut sql = Sql::default();
        sql.push(&"x".repeat(MAX_DESCRIPTION_TEXT_BYTES - 1))
            .unwrap();
        assert_eq!(sql.push("é"), Err(CatalogError::DescriptionLimit));
        assert_eq!(sql.finish().len(), MAX_DESCRIPTION_TEXT_BYTES - 1);
        let mut sql = Sql::default();
        sql.ident("dotted.\"name").unwrap();
        sql.push(" ").unwrap();
        sql.literal("a\\b'c").unwrap();
        assert_eq!(sql.finish(), "\"dotted.\"\"name\" E'a\\\\b''c'");
    }
}

use super::*;
use serde::Serialize;
use std::io::{self, Write};
pub(super) fn name(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_DDL_EXPORT_NAME_BYTES && !value.contains('\0')
}
fn scope_heap(scope: &DdlExportScope) -> Option<usize> {
    match scope {
        DdlExportScope::Database => Some(0),
        DdlExportScope::Schema {
            name: value,
            expected_oid,
        } if name(value) && *expected_oid != Some(0) => Some(value.capacity()),
        DdlExportScope::Relation {
            schema,
            name: value,
            expected,
        } if name(schema)
            && name(value)
            && expected.is_none_or(|id| id.database_oid != 0 && id.relation_oid != 0) =>
        {
            schema.capacity().checked_add(value.capacity())
        }
        _ => None,
    }
}
impl DdlExportRequest {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.expected_database_oid == Some(0) {
            return None;
        }
        let bytes = size_of::<Self>().checked_add(scope_heap(&self.scope)?)?;
        (bytes <= 4096).then_some(bytes)
    }
}
struct Counter(usize);
impl Write for Counter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(buf.len())
            .filter(|n| *n <= MAX_DDL_EXPORT_ENCODED_BYTES)
            .ok_or_else(|| io::Error::other("DDL export bound"))?;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl DdlExportArtifact {
    pub fn encoded_bytes(&self) -> Option<usize> {
        let mut count = Counter(0);
        serde_json::to_writer(&mut count, self).ok()?;
        Some(count.0)
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.connection_id.is_empty()
            || self.connection_id.len() > 256
            || self.connection_id.contains('\0')
            || !name(&self.database)
            || self.database_oid == 0
            || self.reader_pid <= 0
            || self.schemas.len() > MAX_DDL_EXPORT_SCHEMAS
            || self.relations.len() > MAX_DDL_EXPORT_RELATIONS
            || self.sql.len() > MAX_DDL_EXPORT_SQL_BYTES
            || self.sql.contains('\0')
            || self.omissions != types::OMISSIONS
            || self
                .request
                .expected_database_oid
                .is_some_and(|id| id != self.database_oid)
        {
            return None;
        }
        if [&self.collected_start, &self.collected_end]
            .iter()
            .any(|value| value.len() > 64 || chrono::DateTime::parse_from_rfc3339(value).is_err())
        {
            return None;
        }
        let mut bytes = size_of::<Self>()
            .checked_add(
                self.request
                    .checked_heap_bytes()?
                    .checked_sub(size_of::<DdlExportRequest>())?,
            )?
            .checked_add(self.connection_id.capacity())?
            .checked_add(self.database.capacity())?
            .checked_add(self.collected_start.capacity())?
            .checked_add(self.collected_end.capacity())?
            .checked_add(self.sql.capacity())?
            .checked_add(
                self.schemas
                    .capacity()
                    .checked_mul(size_of::<DdlExportSchema>())?,
            )?
            .checked_add(
                self.relations
                    .capacity()
                    .checked_mul(size_of::<DdlExportRelation>())?,
            )?
            .checked_add(
                self.omissions
                    .capacity()
                    .checked_mul(size_of::<DdlExportOmission>())?,
            )?;
        for (index, schema) in self.schemas.iter().enumerate() {
            if schema.oid == 0
                || !name(&schema.name)
                || self.schemas[..index]
                    .iter()
                    .any(|prior| prior.oid == schema.oid || prior.name >= schema.name)
            {
                return None;
            }
            let declared = match &self.request.scope {
                DdlExportScope::Database => schema.name != "public",
                DdlExportScope::Schema { .. } => true,
                DdlExportScope::Relation { .. } => false,
            };
            if schema.declared != declared {
                return None;
            }
            bytes = bytes.checked_add(schema.name.capacity())?;
        }
        match &self.request.scope {
            DdlExportScope::Database => {}
            DdlExportScope::Schema { name, expected_oid } => {
                if self.schemas.len() != 1
                    || self.schemas[0].name != *name
                    || expected_oid.is_some_and(|id| id != self.schemas[0].oid)
                {
                    return None;
                }
            }
            DdlExportScope::Relation {
                schema,
                name,
                expected,
            } => {
                if self.relations.len() != 1 || self.schemas.len() != 1 {
                    return None;
                }
                let r = &self.relations[0];
                if r.schema != *schema
                    || r.name != *name
                    || expected.is_some_and(|id| id != r.identity)
                {
                    return None;
                }
            }
        }
        let mut end = 0;
        for (index, r) in self.relations.iter().enumerate() {
            if !name(&r.schema)
                || !name(&r.name)
                || r.identity.database_oid != self.database_oid
                || r.identity.relation_oid == 0
                || !self
                    .schemas
                    .iter()
                    .any(|schema| schema.oid == r.schema_oid && schema.name == r.schema)
                || r.sql_start < end
                || r.sql_start >= r.sql_end
                || self.relation_sql(index).is_none()
                || self.relations[..index].iter().any(|prior| {
                    prior.identity == r.identity
                        || (&prior.schema, &prior.name) >= (&r.schema, &r.name)
                })
            {
                return None;
            }
            end = r.sql_end;
            bytes = bytes
                .checked_add(r.schema.capacity())?
                .checked_add(r.name.capacity())?;
        }
        if bytes > MAX_DDL_EXPORT_HEAP_BYTES {
            return None;
        }
        self.encoded_bytes()?;
        Some(bytes)
    }
}

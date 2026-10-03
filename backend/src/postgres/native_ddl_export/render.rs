use super::*;
/// Space for bounded relation/schema identity arrays, timestamps and omissions.
/// SQL JSON escaping is charged separately before each append.
const HEADER_ENCODED_ALLOWANCE: usize = 1024 * 1024;
pub(super) struct Sql {
    text: String,
    encoded: usize,
}
impl Sql {
    pub(super) fn new() -> Result<Self, CatalogError> {
        let mut sql = Self {
            text: String::new(),
            encoded: 2,
        };
        sql.push("-- Relation-oriented reconstruction; not a canonical database backup.\n")?;
        for omission in types::OMISSIONS {
            sql.push("-- ")?;
            sql.push(omission.explanation())?;
            sql.push("\n")?;
        }
        sql.push("\n")?;
        Ok(sql)
    }
    pub(super) fn push(&mut self, value: &str) -> Result<(), CatalogError> {
        if value.contains('\0')
            || value.len() > MAX_DDL_EXPORT_SQL_BYTES.saturating_sub(self.text.len())
        {
            return Err(CatalogError::DdlExportLimit);
        }
        let encoded = escaped_bytes(value).ok_or(CatalogError::DdlExportLimit)?;
        if encoded
            > MAX_DDL_EXPORT_ENCODED_BYTES
                .saturating_sub(HEADER_ENCODED_ALLOWANCE)
                .saturating_sub(self.encoded)
        {
            return Err(CatalogError::DdlExportLimit);
        }
        // Grow geometrically within the SQL cap, avoiding a whole-artifact
        // reallocation for every small schema/separator append.
        let required = self.text.len() + value.len();
        if required > self.text.capacity() {
            let target = required
                .max(self.text.capacity().saturating_mul(2))
                .min(MAX_DDL_EXPORT_SQL_BYTES);
            self.text
                .try_reserve_exact(target - self.text.len())
                .map_err(|_| CatalogError::DdlExportLimit)?;
        }
        if self.text.capacity() > MAX_DDL_EXPORT_HEAP_BYTES.saturating_sub(HEADER_ENCODED_ALLOWANCE)
        {
            return Err(CatalogError::DdlExportLimit);
        }
        self.text.push_str(value);
        self.encoded += encoded;
        Ok(())
    }
    pub(super) fn schema(&mut self, schema: &DdlExportSchema) -> Result<(), CatalogError> {
        if schema.declared {
            self.push("CREATE SCHEMA IF NOT EXISTS ")?;
            self.push(&crate::quote_double(&schema.name))?;
            self.push(";\n\n")?;
        }
        Ok(())
    }
    pub(super) fn relation(
        &mut self,
        relation: &mut DdlExportRelation,
        text: &str,
    ) -> Result<(), CatalogError> {
        relation.sql_start = self.text.len() as u32;
        self.push(text)?;
        relation.sql_end = self.text.len() as u32;
        self.push("\n\n")?;
        Ok(())
    }
    pub(super) fn finish(self) -> String {
        self.text
    }
}
fn escaped_bytes(text: &str) -> Option<usize> {
    text.chars().try_fold(0usize, |sum, ch| {
        sum.checked_add(match ch {
            '"' | '\\' | '\n' | '\r' | '\t' | '\x08' | '\x0c' => 2,
            '\0'..='\x1f' => 6,
            _ => ch.len_utf8(),
        })
    })
}

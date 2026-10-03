//! Original-source duplication and atomic selected-row assignment.
use super::*;

#[derive(Debug, PartialEq, Eq)]
pub struct BulkOutcome {
    pub selected: usize,
    pub changed: usize,
}
pub type BulkRow<'a> = (&'a [Option<String>], Option<&'a [String]>);

impl MutationDraft {
    /// Copies original source values, not a staged overlay. Missing columns are
    /// omitted for defaults; writable keys/serial/BY DEFAULT values stay exact.
    /// No row identity is needed to prepare an insert on an insertable table.
    pub fn duplicate_values(
        &self,
        table_index: usize,
        row: &[Option<String>],
        truncated: bool,
    ) -> Result<Vec<MutationValue>, ModelError> {
        self.editable()?;
        let table = self.table(table_index)?;
        let analysis = self.analysis.as_ref().ok_or(ModelError::Stale)?;
        if !table.insertable.allowed || truncated || row.len() != analysis.columns.len() {
            return Err(ModelError::Unavailable);
        }
        // Bound all cloning before materializing selected insert values.
        if crate::results::encoded_size(&row) > DRAFT_BYTES {
            return Err(ModelError::Budget);
        }
        let mut result: Vec<MutationValue> = Vec::new();
        for (column, value) in analysis.columns.iter().zip(row) {
            let ColumnOrigin::Table {
                schema,
                table: name,
                column: original,
                ..
            } = &column.origin
            else {
                continue;
            };
            if schema != &table.schema
                || name != &table.table
                || column.writability != ColumnWritability::Writable
            {
                continue;
            }
            if let Some(old) = result.iter().find(|old| old.column == *original) {
                if &old.value != value {
                    return Err(ModelError::InvalidInput);
                }
            } else {
                result.push(MutationValue {
                    column: original.clone(),
                    value: value.clone(),
                });
            }
        }
        if crate::results::encoded_size(&result) > DRAFT_BYTES {
            return Err(ModelError::Budget);
        }
        Ok(result)
    }

    /// Uses original rows/hidden identity even when an earlier edit changes a
    /// key. Every selected row must be eligible; errors preserve the entire draft.
    /// Source captures and repeated assignment text are separately premeasured
    /// before a single candidate clone, bounding temporary work as well as output.
    pub fn stage_bulk_update(
        &mut self,
        table_index: usize,
        rows: &[BulkRow<'_>],
        truncated: bool,
        edit: MutationValue,
    ) -> Result<BulkOutcome, ModelError> {
        self.editable()?;
        if rows.is_empty() {
            return Err(ModelError::InvalidInput);
        }
        if rows.len() > CHANGE_LIMIT {
            return Err(ModelError::Budget);
        }
        let table = self.table(table_index)?;
        if !table.updatable.allowed || truncated {
            return Err(ModelError::Unavailable);
        }
        self.writes(table, std::slice::from_ref(&edit))?;
        let mut input_bytes = crate::results::encoded_size(&edit).saturating_mul(rows.len());
        for (row, hidden) in rows {
            input_bytes = input_bytes
                .saturating_add(crate::results::encoded_size(row))
                .saturating_add(crate::results::encoded_size(hidden));
            if input_bytes > DRAFT_BYTES {
                return Err(ModelError::Budget);
            }
        }
        let mut changes = self.changes.clone();
        let mut changed = 0;
        for (row, hidden) in rows {
            changed += usize::from(self.update_changes(
                &mut changes,
                table_index,
                row,
                *hidden,
                vec![edit.clone()],
            )?);
            if changes.len() > CHANGE_LIMIT || crate::results::encoded_size(&changes) > DRAFT_BYTES
            {
                return Err(ModelError::Budget);
            }
        }
        if changed != 0 {
            self.publish(changes)?;
        }
        Ok(BulkOutcome {
            selected: rows.len(),
            changed,
        })
    }
}

#[cfg(test)]
mod tests;

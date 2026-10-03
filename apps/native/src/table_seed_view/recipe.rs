use super::*;
use std::mem::size_of;
const DRAFT_BYTES: usize = 512 * 1024;
pub(super) struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub generated: bool,
    pub identity: bool,
    pub has_default: bool,
}
pub(super) struct SetupAnchor {
    pub attempt: TableSeedAttemptId,
    pub endpoint: TableSeedEndpoint,
    pub row_count: u32,
    pub seed: Option<u64>,
}
impl SetupAnchor {
    pub fn matches(&self, intent: &TableSeedIntent) -> bool {
        self.endpoint == intent.endpoint
            && self.row_count == intent.row_count
            && self.seed == intent.seed
    }
}
pub(super) struct Recipe {
    pub attempt: TableSeedAttemptId,
    pub endpoint: TableSeedEndpoint,
    pub row_count: u32,
    pub seed: Option<u64>,
    pub columns: Vec<Column>,
    pub drafts: Vec<ColumnDraft>,
    pub changed: bool,
    // Once the original preparation is explicitly settled, retry need not
    // address a released attempt. This never makes a new recipe executable.
    pub discarded: bool,
}
impl Recipe {
    pub fn capture(inspection: &TableSeedInspection) -> Result<Self, &'static str> {
        let columns = inspection.columns();
        let specs = inspection.specs();
        if columns.len() > MAX_TABLE_SEED_COLUMNS
            || inspection.retained_bytes() > MAX_TABLE_SEED_REVIEW_BYTES
        {
            return Err("Seed column metadata exceeds native bounds");
        }
        let mut estimate = columns
            .len()
            .checked_mul(size_of::<Column>() + size_of::<ColumnDraft>())
            .ok_or("Recipe size overflow")?;
        for column in columns {
            if !valid_name(&column.name) {
                return Err("Invalid inspected column identity");
            }
            estimate = estimate
                .checked_add(column.name.len())
                .and_then(|n| n.checked_add(column.data_type.len()))
                .ok_or("Recipe size overflow")?;
        }
        for spec in specs {
            estimate=estimate.checked_add(match &spec.source {
                TableSeedSource::Constant{value}=>value.len(),
                TableSeedSource::Values{values}=>{
                    if values.iter().any(|value|value.contains(',')||value.trim()!=value||value.is_empty()) {return Err("This exact value list cannot be represented by comma-separated editing; accepted review is unchanged");}
                    values.iter().try_fold(values.len(),|n,value|n.checked_add(value.len())).ok_or("Recipe size overflow")?
                }
                _=>0,
            }).and_then(|n|n.checked_add(32)).ok_or("Recipe size overflow")?;
        }
        if estimate > DRAFT_BYTES {
            return Err("Editable column recipe exceeds 512 KiB; accepted review is unchanged");
        }
        let mut captured = Vec::with_capacity(columns.len());
        let mut drafts = Vec::with_capacity(columns.len());
        for column in columns {
            captured.push(Column {
                name: column.name.clone(),
                data_type: column.data_type.clone(),
                nullable: column.nullable,
                generated: column.generated,
                identity: column.identity,
                has_default: column.has_default,
            });
            let mut draft = ColumnDraft::default();
            if let Some(spec) = specs.iter().find(|spec| spec.column == column.name) {
                draft.mode = match &spec.source {
                    TableSeedSource::Auto { generator: None } => ColumnMode::Auto,
                    TableSeedSource::Auto {
                        generator: Some(generator),
                    } => ColumnMode::Generator(*generator),
                    TableSeedSource::Default => ColumnMode::Default,
                    TableSeedSource::Constant { value } => {
                        draft.constant = value.clone();
                        ColumnMode::Constant
                    }
                    TableSeedSource::Values { values } => {
                        draft.values_text = values.join(",");
                        ColumnMode::Values
                    }
                };
                draft.null_percent = spec.null_rate.map_or_else(String::new, percent_text);
            }
            drafts.push(draft);
        }
        let recipe = Self {
            attempt: inspection.attempt_id(),
            endpoint: inspection.intent().endpoint.clone(),
            row_count: inspection.intent().row_count,
            seed: inspection.intent().seed,
            columns: captured,
            drafts,
            changed: false,
            discarded: false,
        };
        if recipe.heap_bytes() > DRAFT_BYTES {
            return Err("Editable column recipe exceeds 512 KiB");
        }
        Ok(recipe)
    }
    fn heap_bytes(&self) -> usize {
        self.columns.capacity() * size_of::<Column>()
            + self.drafts.capacity() * size_of::<ColumnDraft>()
            + self
                .columns
                .iter()
                .map(|column| column.name.capacity() + column.data_type.capacity())
                .sum::<usize>()
            + self.drafts.iter().map(draft_bytes).sum::<usize>()
    }
    pub fn save(&mut self, index: usize, draft: ColumnDraft) -> Result<(), &'static str> {
        let column = self.columns.get(index).ok_or("Column is unavailable")?;
        // Check capacity before even validating (validation creates a bounded spec).
        let old = self.drafts.get(index).ok_or("Column is unavailable")?;
        if self
            .heap_bytes()
            .saturating_sub(draft_bytes(old))
            .saturating_add(draft_bytes(&draft))
            > DRAFT_BYTES
        {
            return Err("Editable column recipes exceed 512 KiB; previous value retained");
        }
        let _ = draft.spec(&column.name, column.nullable)?;
        let changed = old.mode != draft.mode
            || old.constant != draft.constant
            || old.values_text != draft.values_text
            || old.null_percent != draft.null_percent;
        self.drafts[index] = draft;
        self.changed |= changed;
        Ok(())
    }
    pub fn specs(&self) -> Result<Vec<TableSeedColumnSpec>, &'static str> {
        let selected = self
            .columns
            .iter()
            .zip(&self.drafts)
            .filter(|(column, draft)| draft.overridden(column.nullable));
        let count = selected.clone().count();
        let mut bytes = count
            .checked_mul(size_of::<TableSeedColumnSpec>())
            .ok_or("Recipe size overflow")?;
        for (column, draft) in selected.clone() {
            let values = if draft.mode == ColumnMode::Values {
                draft
                    .values_text
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .count()
            } else {
                0
            };
            let literal_bytes = match draft.mode {
                ColumnMode::Constant => draft.constant.len(),
                ColumnMode::Values => draft.values_text.len(),
                _ => 0,
            };
            bytes = bytes
                .checked_add(column.name.len())
                .and_then(|n| n.checked_add(literal_bytes))
                .and_then(|n| n.checked_add(values.checked_mul(size_of::<String>())?))
                .ok_or("Recipe size overflow")?;
        }
        if bytes > MAX_TABLE_SEED_SPEC_BYTES {
            return Err(
                "Seed recipe exceeds the 128 KiB request allowance; reduce overrides or literal values",
            );
        }
        let mut specs = Vec::with_capacity(count);
        for (column, draft) in selected {
            specs.push(draft.spec(&column.name, column.nullable)?);
        }
        Ok(specs)
    }
}
fn draft_bytes(draft: &ColumnDraft) -> usize {
    draft.constant.capacity() + draft.values_text.capacity() + draft.null_percent.capacity()
}
pub(super) fn mode_at(index: usize) -> Option<ColumnMode> {
    match index {
        0 => Some(ColumnMode::Auto),
        1 => Some(ColumnMode::Default),
        2 => Some(ColumnMode::Constant),
        3 => Some(ColumnMode::Values),
        _ => GENERATORS
            .get(index - 4)
            .map(|(generator, _)| ColumnMode::Generator(*generator)),
    }
}
pub(super) fn mode_label(mode: ColumnMode) -> &'static str {
    match mode {
        ColumnMode::Auto => "Auto",
        ColumnMode::Default => "Skip (use DEFAULT)",
        ColumnMode::Constant => "Constant",
        ColumnMode::Values => "Value list",
        ColumnMode::Generator(generator) => GENERATORS
            .iter()
            .find(|(item, _)| *item == generator)
            .map_or("Generator", |(_, label)| label),
    }
}

// Decimal Display expands subnormal rates into hundreds of zeros. Scientific
// notation keeps every finite reviewed percentage within the bounded editor.
pub(super) fn percent_text(rate: f64) -> String {
    let percent = rate * 100.;
    let text = percent.to_string();
    if text.len() <= 32 {
        text
    } else {
        format!("{percent:e}")
    }
}

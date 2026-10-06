//! Staged values painted over a browse page. Rows are matched by captured
//! identity, never by position, so tints follow their rows across paging,
//! sorting and filtering. Building needs no analysis: an invalidated draft
//! still paints while the next page is analysed.
use super::*;
use std::collections::HashMap;

/// Staged text shown in a cell is capped; the editor reloads the full value.
pub const OVERLAY_TEXT_BYTES: usize = 2048;

/// A grid cell target: a page row, or a row of the insert band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CellRef {
    Page(usize),
    Insert(Uuid),
}

/// How an editor opens: with the current value, replaced by a typed
/// character, or cleared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditSeed {
    Keep,
    Replace(String),
    Clear,
}

/// Where the cursor goes after an edit is staged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Advance {
    Stay,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayValue {
    /// `None` is SQL NULL.
    pub text: Option<String>,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowMark {
    Updated {
        change: Uuid,
        included: bool,
        /// `(page column index, staged value)`, ascending by column.
        cells: Vec<(usize, OverlayValue)>,
    },
    Deleted {
        change: Uuid,
        included: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InsertCell {
    /// Omitted from the insert, so the column default applies.
    Default,
    Value(OverlayValue),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertRow {
    pub change: Uuid,
    pub included: bool,
    /// One cell per page column.
    pub cells: Vec<InsertCell>,
}

/// Cache key: the draft's `(owner, revision)` and the page `Rc` address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayKey {
    pub draft: Option<(Uuid, u64)>,
    pub page: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftOverlay {
    pub key: OverlayKey,
    /// One entry per page row.
    pub rows: Vec<Option<RowMark>>,
    pub inserts: Vec<InsertRow>,
    /// The change named by the last apply failure.
    pub failed: Option<Uuid>,
}

impl DraftOverlay {
    /// No staged changes over a page of `page_rows` rows.
    pub fn empty(page_rows: usize) -> Self {
        Self {
            key: OverlayKey {
                draft: None,
                page: 0,
            },
            rows: vec![None; page_rows],
            inserts: Vec::new(),
            failed: None,
        }
    }

    pub fn mark(&self, row: usize) -> Option<&RowMark> {
        self.rows.get(row).and_then(Option::as_ref)
    }

    /// The staged value of an updated cell; `source` is the page column.
    pub fn cell(&self, row: usize, source: usize) -> Option<&OverlayValue> {
        match self.mark(row)? {
            RowMark::Updated { cells, .. } => cells
                .iter()
                .find(|(column, _)| *column == source)
                .map(|(_, value)| value),
            RowMark::Deleted { .. } => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inserts.is_empty() && self.rows.iter().all(Option::is_none)
    }
}

fn overlay_value(value: &Option<String>) -> OverlayValue {
    match value {
        None => OverlayValue {
            text: None,
            truncated: false,
        },
        Some(text) if text.len() <= OVERLAY_TEXT_BYTES => OverlayValue {
            text: Some(text.clone()),
            truncated: false,
        },
        Some(text) => {
            let mut end = OVERLAY_TEXT_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            OverlayValue {
                text: Some(text[..end].to_owned()),
                truncated: true,
            }
        }
    }
}

/// Page rows indexed by one identity shape (column names plus whether the
/// hidden ctid identity applies). Unproven keys may repeat, so each identity
/// keeps every row that carries it.
type IdentityIndex = HashMap<Vec<Option<String>>, Vec<usize>>;

fn identity_index(
    page: &BrowseTableResult,
    names: &HashMap<&str, usize>,
    columns: &[String],
    ctid: bool,
) -> IdentityIndex {
    let mut index = IdentityIndex::new();
    for (row, values) in page.rows.iter().enumerate() {
        let hidden = if ctid {
            page.row_identity.as_ref().map(|rows| rows.get(row))
        } else {
            None
        };
        let identity = match hidden {
            Some(Some(hidden)) if hidden.len() == columns.len() => {
                Some(hidden.iter().cloned().map(Some).collect::<Vec<_>>())
            }
            Some(_) => None,
            None => columns
                .iter()
                .map(|column| names.get(column.as_str()).and_then(|&i| values.get(i)).cloned())
                .collect::<Option<Vec<_>>>(),
        };
        if let Some(identity) = identity {
            index.entry(identity).or_default().push(row);
        }
    }
    index
}

/// Unproven identities (virtual keys, reused ctids) mark a row only when its
/// page values still equal the captured originals, as `existing_in` merges.
fn originals_match(
    row: &CapturedRow,
    values: &[Option<String>],
    names: &HashMap<&str, usize>,
) -> bool {
    row.originals.iter().all(|original| {
        match names.get(original.column.as_str()) {
            Some(&index) => values.get(index) == Some(&original.value),
            // Hidden identity (ctid) was already matched through the index.
            None => row
                .identity
                .iter()
                .any(|identity| identity.column == original.column),
        }
    })
}

fn same_table(target: &MutationTable, schema: &str, table: &str) -> bool {
    target.schema == schema && target.table == table
}

impl MutationDraft {
    /// The table a browse page shows: the single origin of the analysed
    /// projection, or without analysis the one table every change targets.
    fn overlay_table(&self) -> Option<MutationTable> {
        let mut found: Option<MutationTable> = None;
        if let Some(analysis) = &self.analysis {
            for column in &analysis.columns {
                if let ColumnOrigin::Table { schema, table, .. } = &column.origin {
                    match found.as_ref().map(|found| same_table(found, schema, table)) {
                        Some(true) => {}
                        Some(false) => return None,
                        None => {
                            found = Some(MutationTable {
                                schema: schema.clone(),
                                table: table.clone(),
                            })
                        }
                    }
                }
            }
            return found;
        }
        for change in &self.changes {
            let target = match &change.operation {
                MutationOp::Insert { table, .. }
                | MutationOp::Update { table, .. }
                | MutationOp::Delete { table, .. } => table,
            };
            match found.as_ref().map(|found| found == target) {
                Some(true) => {}
                Some(false) => return None,
                None => found = Some(target.clone()),
            }
        }
        found
    }

    /// Paints staged changes over `page`. `page_key` identifies the page for
    /// caching (its `Rc` address); `failed` is the last apply failure's change.
    pub fn overlay(
        &self,
        page: &BrowseTableResult,
        page_key: usize,
        failed: Option<Uuid>,
    ) -> DraftOverlay {
        let mut overlay = DraftOverlay {
            key: OverlayKey {
                draft: Some((self.owner, self.revision)),
                page: page_key,
            },
            rows: vec![None; page.rows.len()],
            inserts: Vec::new(),
            failed,
        };
        let Some(target) = self.overlay_table() else {
            return overlay;
        };
        let mut names: HashMap<&str, usize> = HashMap::new();
        for (index, column) in page.columns.iter().enumerate() {
            names.entry(column.name.as_str()).or_insert(index);
        }
        let mut indexes: HashMap<(Vec<String>, bool), IdentityIndex> = HashMap::new();
        for change in &self.changes {
            match &change.operation {
                MutationOp::Insert { table, values } => {
                    if table != &target {
                        continue;
                    }
                    overlay.inserts.push(InsertRow {
                        change: change.id,
                        included: change.included,
                        cells: page
                            .columns
                            .iter()
                            .map(|column| {
                                values
                                    .iter()
                                    .find(|value| value.column == column.name)
                                    .map_or(InsertCell::Default, |value| {
                                        InsertCell::Value(overlay_value(&value.value))
                                    })
                            })
                            .collect(),
                    });
                }
                MutationOp::Update { table, .. } | MutationOp::Delete { table, .. } => {
                    let Some(captured) = change.row.as_ref() else {
                        continue;
                    };
                    if table != &target || captured.table != target {
                        continue;
                    }
                    let ctid = captured.kind == MutationIdentityKind::CtidFallback;
                    let unproven = matches!(
                        captured.kind,
                        MutationIdentityKind::VirtualKey | MutationIdentityKind::CtidFallback
                    );
                    let columns = captured
                        .identity
                        .iter()
                        .map(|value| value.column.clone())
                        .collect::<Vec<_>>();
                    let identity = captured
                        .identity
                        .iter()
                        .map(|value| value.value.clone())
                        .collect::<Vec<_>>();
                    let index = indexes
                        .entry((columns, ctid))
                        .or_insert_with_key(|(columns, ctid)| {
                            identity_index(page, &names, columns, *ctid)
                        });
                    let Some(rows) = index.get(&identity) else {
                        continue;
                    };
                    for &row in rows {
                        if overlay.rows[row].is_some()
                            || (unproven && !originals_match(captured, &page.rows[row], &names))
                        {
                            continue;
                        }
                        overlay.rows[row] = Some(match &change.operation {
                            MutationOp::Update { set, .. } => {
                                let mut cells = Vec::new();
                                for value in set {
                                    for (index, column) in page.columns.iter().enumerate() {
                                        if column.name == value.column {
                                            cells.push((index, overlay_value(&value.value)));
                                        }
                                    }
                                }
                                cells.sort_by_key(|(index, _)| *index);
                                RowMark::Updated {
                                    change: change.id,
                                    included: change.included,
                                    cells,
                                }
                            }
                            _ => RowMark::Deleted {
                                change: change.id,
                                included: change.included,
                            },
                        });
                    }
                }
            }
        }
        overlay
    }
}

#[cfg(test)]
mod tests;

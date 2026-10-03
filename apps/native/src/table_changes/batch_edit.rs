//! Bulk and duplicate reuse the existing literal/insert editors and staging path.
use super::*;
use crate::data_model::{BulkOutcome, BulkRow, ModelError};
use serde::{Serialize, Serializer, ser::SerializeMap};

pub(super) struct Capture {
    page: Rc<BrowseTableResult>,
    analysis_id: u64,
}
pub(super) struct BulkEdit {
    capture: Capture,
    rows: Vec<usize>,
    column: usize,
}
pub(super) enum EditContext {
    Ordinary,
    Duplicate(Capture),
    Bulk(BulkEdit),
}
impl EditContext {
    pub(super) fn current(
        &self,
        page: Option<&Rc<BrowseTableResult>>,
        analysis: Option<&AnalyzeResultSetResult>,
    ) -> bool {
        let capture = match self {
            Self::Ordinary => return true,
            Self::Duplicate(capture) => capture,
            Self::Bulk(bulk) => &bulk.capture,
        };
        page.is_some_and(|page| Rc::ptr_eq(page, &capture.page))
            && analysis.is_some_and(|analysis| analysis.analysis_id == capture.analysis_id)
    }
    pub(super) fn label(&self) -> Option<String> {
        match self {
            Self::Ordinary => None,
            Self::Duplicate(_) => Some("Duplicate row JSON, omitted columns use defaults".into()),
            Self::Bulk(bulk) => Some(format!("Set {} selected rows", bulk.rows.len())),
        }
    }
}

fn source_column<'a>(
    analysis: &'a AnalyzeResultSetResult,
    relation: &MutationTable,
    index: usize,
) -> Option<&'a str> {
    let column = analysis.columns.get(index)?;
    if column.writability != ColumnWritability::Writable {
        return None;
    }
    match &column.origin {
        ColumnOrigin::Table {
            schema,
            table,
            column,
            ..
        } if schema == &relation.schema && table == &relation.table => Some(column),
        _ => None,
    }
}

/// JSON strings are SQL text; NULL is explicit and absent keys request defaults.
struct InsertJson<'a>(&'a [MutationValue]);
impl Serialize for InsertJson<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.0.len()))?;
        for value in self.0 {
            map.serialize_entry(&value.column, &value.value)?;
        }
        map.end()
    }
}
fn duplicate_json(values: &[MutationValue]) -> Result<String, ModelError> {
    let json = InsertJson(values);
    if crate::results::encoded_size(&json) > cell_value::MAX_VALUE_BYTES {
        return Err(ModelError::Budget);
    }
    serde_json::to_string(&json).map_err(|_| ModelError::InvalidInput)
}

impl TableChanges {
    pub fn duplicate(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_edit() || self.edit.is_some() {
            return;
        }
        if !self.admit_work() {
            cx.notify();
            return;
        }
        let prepared = (|| {
            let page = self.page.as_ref().ok_or(ModelError::Unavailable)?;
            let values = page.rows.get(row).ok_or(ModelError::InvalidInput)?;
            let values = self
                .draft
                .as_ref()
                .ok_or(ModelError::Unavailable)?
                .duplicate_values(0, values, page.truncated_cells > 0 || page.omitted_rows > 0)?;
            let text = duplicate_json(&values)?;
            let capture = Capture {
                page: page.clone(),
                analysis_id: self.analysis.as_ref().ok_or(ModelError::Stale)?.analysis_id,
            };
            Ok::<_, ModelError>((text, capture))
        })();
        match prepared {
            Ok((text, capture)) => {
                self.open_edit((None, None, 0), text, false, window, cx);
                self.edit.as_mut().unwrap().context = EditContext::Duplicate(capture);
                self.relabel_batch(cx);
                self.message = "Original row copied; omitted columns use defaults".into();
            }
            Err(error) => self.message = format!("Duplicate refused: {error:?}"),
        }
        self.finish_work();
        cx.notify();
    }

    pub fn bulk_edit(
        &mut self,
        mut rows: Vec<usize>,
        column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit() || self.edit.is_some() {
            return;
        }
        if !self.admit_work() {
            cx.notify();
            return;
        }
        rows.sort_unstable();
        rows.dedup();
        let prepared = (|| {
            if rows.is_empty() {
                return Err(ModelError::InvalidInput);
            }
            if rows.len() > 128 {
                return Err(ModelError::Budget);
            }
            let page = self.page.as_ref().ok_or(ModelError::Unavailable)?;
            if page.truncated_cells > 0 || page.omitted_rows > 0 {
                return Err(ModelError::Unavailable);
            }
            if rows.iter().any(|row| *row >= page.rows.len()) {
                return Err(ModelError::InvalidInput);
            }
            let analysis = self.analysis.as_ref().ok_or(ModelError::Stale)?;
            if !analysis
                .tables
                .first()
                .is_some_and(|table| table.updatable.allowed)
            {
                return Err(ModelError::Unavailable);
            }
            let column = if source_column(
                analysis,
                self.source.relation().expect("table batch controls"),
                column,
            )
            .is_some()
            {
                column
            } else {
                (0..analysis.columns.len())
                    .find(|index| {
                        source_column(
                            analysis,
                            self.source.relation().expect("table batch controls"),
                            *index,
                        )
                        .is_some()
                    })
                    .ok_or(ModelError::Unavailable)?
            };
            let name = source_column(
                analysis,
                self.source.relation().expect("table batch controls"),
                column,
            )
            .unwrap()
            .to_owned();
            Ok((
                name,
                BulkEdit {
                    capture: Capture {
                        page: page.clone(),
                        analysis_id: analysis.analysis_id,
                    },
                    rows,
                    column,
                },
            ))
        })();
        match prepared {
            Ok((name, bulk)) => {
                self.open_edit(
                    (Some(bulk.rows[0]), Some(name), 0),
                    String::new(),
                    false,
                    window,
                    cx,
                );
                self.edit.as_mut().unwrap().context = EditContext::Bulk(bulk);
                self.relabel_batch(cx);
                self.message = "Assign one literal value or NULL to the selected rows".into();
            }
            Err(error) => self.message = format!("Bulk edit refused: {error:?}"),
        }
        self.finish_work();
        cx.notify();
    }

    pub(super) fn relabel_batch(&mut self, cx: &mut Context<Self>) {
        let edit = self.edit.as_mut().unwrap();
        if matches!(edit.context, EditContext::Ordinary) {
            return;
        }
        let label = match &edit.context {
            EditContext::Bulk(bulk) => format!(
                "Value for {} in {} selected rows",
                edit.column.as_deref().unwrap_or("column"),
                bulk.rows.len()
            ),
            _ => edit.context.label().unwrap_or_default(),
        };
        let editor = edit.editor.clone();
        let multiline = edit.kind.is_some() || edit.row.is_none();
        edit.accessible = cx.new(|cx| {
            if multiline {
                AccessibleEditor::new(editor, label, cx)
            } else {
                AccessibleEditor::field(editor, label, false, cx)
            }
        });
    }

    pub(super) fn cycle_bulk_column(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composition_active(window, cx) {
            self.message = "Finish composing before changing columns".into();
            return;
        }
        let Some(edit) = &self.edit else {
            return;
        };
        let EditContext::Bulk(bulk) = &edit.context else {
            return;
        };
        if !edit
            .context
            .current(self.page.as_ref(), self.analysis.as_ref())
        {
            self.message = "Source changed; reopen bulk edit".into();
            return;
        }
        let analysis = self.analysis.as_ref().unwrap();
        let next = (1..=analysis.columns.len())
            .map(|offset| (bulk.column + offset) % analysis.columns.len())
            .find(|index| {
                source_column(
                    analysis,
                    self.source.relation().expect("table batch controls"),
                    *index,
                )
                .is_some()
            });
        let Some(next) = next else {
            return;
        };
        let name = source_column(
            analysis,
            self.source.relation().expect("table batch controls"),
            next,
        )
        .unwrap()
        .to_owned();
        if edit.editor.read(cx).buffer().read(cx).len(cx).0 > cell_value::MAX_VALUE_BYTES {
            self.message = "Cell input exceeds 1 MiB; text retained".into();
            return;
        }
        let text = if let Some(array) = edit.array.clone() {
            match array.update(cx, |array, cx| array.replacement(window, cx)) {
                Ok(Some(text)) => text,
                Ok(None) => edit.editor.read(cx).text(cx),
                Err(error) => {
                    self.message = error;
                    return;
                }
            }
        } else {
            edit.editor.read(cx).text(cx)
        };
        let null = edit.null;
        let mut edit = self.edit.take().unwrap();
        let EditContext::Bulk(mut bulk) =
            std::mem::replace(&mut edit.context, EditContext::Ordinary)
        else {
            unreachable!();
        };
        bulk.column = next;
        // Column selection must not replace a retained literal just because
        // NULL is selected (the ordinary empty-array initializer is cell-only).
        self.open_edit((Some(bulk.rows[0]), Some(name), 0), text, false, window, cx);
        let edit = self.edit.as_mut().unwrap();
        edit.null = null;
        edit.context = EditContext::Bulk(bulk);
        self.relabel_batch(cx);
    }

    pub(super) fn stage_bulk(&mut self, text: String, cx: &mut Context<Self>) {
        let edit = self.edit.as_ref().unwrap();
        let EditContext::Bulk(bulk) = &edit.context else {
            return;
        };
        let rows: Vec<BulkRow<'_>> = bulk
            .rows
            .iter()
            .map(|index| {
                (
                    bulk.capture.page.rows[*index].as_slice(),
                    bulk.capture
                        .page
                        .row_identity
                        .as_ref()
                        .and_then(|rows| rows.get(*index))
                        .map(Vec::as_slice),
                )
            })
            .collect::<Vec<_>>();
        let value = MutationValue {
            column: edit.column.clone().unwrap(),
            value: if edit.null { None } else { Some(text) },
        };
        let result: Result<BulkOutcome, ModelError> = self
            .draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| draft.stage_bulk_update(0, &rows, false, value));
        match result {
            Ok(outcome) => {
                self.return_focus(cx);
                self.edit = None;
                if outcome.changed > 0 {
                    self.discard_review();
                    cx.emit(ChangesEvent::Changed);
                }
                self.message = format!(
                    "{} of {} selected rows changed; changes are staged",
                    outcome.changed, outcome.selected
                );
            }
            Err(error) => self.message = format!("Bulk edit refused; draft unchanged: {error:?}"),
        }
        self.finish_work();
        cx.notify();
    }
}

#[cfg(test)]
mod tests;

//! Row-level staging: add, duplicate and delete rows, and the bulk editor,
//! which reuses the literal editor and staging path.
use super::*;
use crate::data_model::{BulkOutcome, BulkRow};

pub(super) struct Capture {
    page: Rc<BrowseTableResult>,
    analysis_id: u64,
}
impl Capture {
    /// The captured page and analysis are still the ones on screen.
    pub(super) fn current(
        &self,
        page: Option<&Rc<BrowseTableResult>>,
        analysis: Option<&AnalyzeResultSetResult>,
    ) -> bool {
        page.is_some_and(|page| Rc::ptr_eq(page, &self.page))
            && analysis.is_some_and(|analysis| analysis.analysis_id == self.analysis_id)
    }
}
pub(super) struct BulkEdit {
    capture: Capture,
    rows: Vec<usize>,
    column: usize,
}
pub(super) enum EditContext {
    Ordinary,
    Bulk(BulkEdit),
}
impl EditContext {
    pub(super) fn current(
        &self,
        page: Option<&Rc<BrowseTableResult>>,
        analysis: Option<&AnalyzeResultSetResult>,
    ) -> bool {
        match self {
            Self::Ordinary => true,
            Self::Bulk(bulk) => bulk.capture.current(page, analysis),
        }
    }
    pub(super) fn label(&self) -> Option<String> {
        match self {
            Self::Ordinary => None,
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

impl TableChanges {
    /// Checks shared by the direct row operations (no editor involved).
    fn row_operation_ready(&mut self) -> Result<(), SharedString> {
        if self.is_query() {
            return Err("Rows can only be added or deleted in a table tab".into());
        }
        if self.edit.is_some() {
            return Err("Save or cancel the open edit first".into());
        }
        self.can_edit_now()?;
        if !self.admit_work() {
            return Err(self.message.clone().into());
        }
        Ok(())
    }
    /// Stages an empty insert; every cell shows DEFAULT in the insert band.
    pub fn add_row(&mut self, cx: &mut Context<Self>) -> Result<Uuid, SharedString> {
        self.row_operation_ready()?;
        let result = self
            .draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| draft.stage_insert(0, vec![]));
        let id = result.as_ref().ok().copied();
        self.finish_change("Add row", result.map(|_| ()), cx)?;
        self.message = "Row added; edit its cells, then review".into();
        Ok(id.expect("staged insert id"))
    }
    /// Removes a draft insert (the band's × button).
    pub fn remove_insert(&mut self, change: Uuid, cx: &mut Context<Self>) {
        if self
            .edit
            .as_ref()
            .is_some_and(|edit| edit.insert == Some(change))
        {
            self.close_edit(Advance::Stay, cx);
            self.finish_work();
        }
        if let Err(reason) = self.can_select_now() {
            self.message = reason.to_string();
            cx.notify();
            return;
        }
        if !self.admit_work() {
            cx.notify();
            return;
        }
        let result = self
            .draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| draft.remove(change));
        let _ = self.finish_change("Remove row", result, cx);
    }
    /// Stages deletes for checked page rows, all or nothing.
    pub fn stage_deletes(
        &mut self,
        rows: &[usize],
        cx: &mut Context<Self>,
    ) -> Result<usize, SharedString> {
        if rows.is_empty() {
            return Err("Check the rows to delete first".into());
        }
        self.row_operation_ready()?;
        let Some(page) = self.page.clone() else {
            self.finish_work();
            return Err("The table page is not loaded".into());
        };
        let selected = rows
            .iter()
            .map(|row| {
                page.rows.get(*row).map(|values| {
                    (
                        values.as_slice(),
                        page.row_identity
                            .as_ref()
                            .and_then(|identity| identity.get(*row))
                            .map(Vec::as_slice),
                    )
                })
            })
            .collect::<Option<Vec<BulkRow<'_>>>>();
        let Some(selected) = selected else {
            self.finish_work();
            return Err("A checked row is no longer on this page".into());
        };
        let result = self
            .draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| draft.stage_deletes(0, &selected, page.truncated_cells > 0));
        let count = result.as_ref().ok().copied();
        drop(selected);
        self.finish_change("Delete", result.map(|_| ()), cx)?;
        let count = count.unwrap_or_default();
        self.message = format!(
            "{count} row{} staged for deletion; review to apply",
            if count == 1 { "" } else { "s" }
        );
        Ok(count)
    }
    /// Copies the original source row (not staged values) into a new insert.
    /// Missing columns stay omitted for their defaults.
    pub fn duplicate(&mut self, row: usize, cx: &mut Context<Self>) -> Result<Uuid, SharedString> {
        self.row_operation_ready()?;
        let result = (|| {
            let page = self.page.clone().ok_or(ModelError::Unavailable)?;
            let values = page.rows.get(row).ok_or(ModelError::InvalidInput)?;
            let draft = self.draft.as_mut().ok_or(ModelError::Unavailable)?;
            let values = draft.duplicate_values(
                0,
                values,
                page.truncated_cells > 0 || page.omitted_rows > 0,
            )?;
            draft.stage_insert(0, values)
        })();
        let id = result.as_ref().ok().copied();
        self.finish_change("Duplicate", result.map(|_| ()), cx)?;
        self.message = "Row duplicated into a new insert; review to apply".into();
        Ok(id.expect("staged insert id"))
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
                let (row, column) = (bulk.rows[0], bulk.column);
                self.open_edit(
                    (Some(row), Some(name), 0),
                    String::new(),
                    false,
                    Presentation::Popover,
                    window,
                    cx,
                );
                self.edit.as_mut().unwrap().context = EditContext::Bulk(bulk);
                self.relabel_batch(cx);
                self.message = "Assign one literal value or NULL to the selected rows".into();
                cx.emit(ChangesEvent::EditOpened {
                    cell: CellRef::Page(row),
                    source: column,
                    popover: true,
                });
            }
            Err(error) => {
                self.message = format!("Bulk edit refused: {}", model_error_text(error));
            }
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
        let multiline = edit.multiline();
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
        self.open_edit(
            (Some(bulk.rows[0]), Some(name), 0),
            text,
            false,
            Presentation::Popover,
            window,
            cx,
        );
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
        drop(rows);
        match result {
            Ok(outcome) => {
                self.close_edit(Advance::Stay, cx);
                if outcome.changed > 0 {
                    self.discard_review();
                    cx.emit(ChangesEvent::Changed);
                    cx.emit(ChangesEvent::OverlayChanged);
                }
                self.message = format!(
                    "{} of {} selected rows changed; changes are staged",
                    outcome.changed, outcome.selected
                );
            }
            Err(error) => {
                self.edit_refused(
                    &format!(
                        "Bulk edit refused; draft unchanged: {}",
                        model_error_text(error)
                    ),
                    cx,
                );
            }
        }
        self.finish_work();
        cx.notify();
    }
}

#[cfg(test)]
mod tests;

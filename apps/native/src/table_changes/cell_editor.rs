//! In-cell editing (plan 032 §3.6). Short scalar values edit in a single-line
//! editor the grid hosts over the cell; JSON, arrays, geometry, multi-line and
//! long values open the popover editor. Both stage through `stage()`.
use super::*;
use crate::style;
use gpui::{Keystroke, MouseDownEvent};

/// Values longer than this open the popover editor.
pub(crate) const INLINE_BYTES: usize = 4 * 1024;

/// Applies `seed` to the cell value and picks where the editor opens. The
/// returned text is `None` for an unedited NULL.
pub(crate) fn edit_presentation(
    kind: Option<Kind>,
    value: Option<&str>,
    seed: &EditSeed,
) -> (Presentation, Option<String>) {
    let seeded = match seed {
        EditSeed::Keep => value.map(str::to_owned),
        EditSeed::Clear => Some(String::new()),
        EditSeed::Replace(text) => Some(text.clone()),
    };
    let popover = kind.is_some()
        || seeded
            .as_deref()
            .is_some_and(|text| text.contains('\n') || text.len() > INLINE_BYTES);
    (
        if popover {
            Presentation::Popover
        } else {
            Presentation::Inline
        },
        seeded,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CellEditorEvent {
    Commit(Advance),
    Cancel,
    Expand,
}

/// Keys the inline editor handles itself. `enter` and `cmd-enter` stage in
/// place, Tab moves, `alt-enter` switches to the popover and Esc cancels.
pub(super) fn cell_editor_key(keystroke: &Keystroke) -> Option<CellEditorEvent> {
    let modifiers = keystroke.modifiers;
    let plain = !modifiers.control && !modifiers.alt && !modifiers.function;
    match keystroke.key.as_str() {
        "enter" if modifiers.alt && !modifiers.control && !modifiers.platform => {
            Some(CellEditorEvent::Expand)
        }
        "enter" if plain && !modifiers.shift => Some(CellEditorEvent::Commit(Advance::Stay)),
        "tab" if plain && !modifiers.platform => Some(CellEditorEvent::Commit(if modifiers.shift {
            Advance::Left
        } else {
            Advance::Right
        })),
        "escape" if !modifiers.modified() => Some(CellEditorEvent::Cancel),
        _ => None,
    }
}

/// The inline editor view the grid renders at the cell. It owns no edit
/// state: TableChanges keeps the draft, history and staging.
pub(super) struct CellEditor {
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
impl EventEmitter<CellEditorEvent> for CellEditor {}
impl CellEditor {
    pub(super) fn new(editor: Entity<Editor>, accessible: Entity<AccessibleEditor>) -> Self {
        Self { editor, accessible }
    }
    /// The literal guard replaced the editor at its size or history limit.
    pub(super) fn replace(
        &mut self,
        editor: Entity<Editor>,
        accessible: Entity<AccessibleEditor>,
        cx: &mut Context<Self>,
    ) {
        self.editor = editor;
        self.accessible = accessible;
        cx.notify();
    }
    fn composing(&self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.editor.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        })
    }
}
impl Focusable for CellEditor {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.editor.focus_handle(cx)
    }
}
impl Render for CellEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("cell-editor")
            .key_context("CellEditor")
            .size_full()
            .flex()
            .items_center()
            .px(px(4.))
            .overflow_hidden()
            .bg(style::bg())
            .border_1()
            .border_color(style::accent())
            .font_family(style::MONO)
            .text_size(px(style::FONT))
            .text_color(style::text())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let Some(outcome) = cell_editor_key(&event.keystroke) else {
                    return;
                };
                // IME composition owns Enter, Tab and Esc until it commits.
                if this.composing(window, cx) {
                    return;
                }
                cx.stop_propagation();
                cx.emit(outcome);
            }))
            // Clicking away stages the edit, so the click still reaches its
            // target (another cell, a toolbar button).
            .on_mouse_down_out(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                if !this.composing(window, cx) {
                    cx.emit(CellEditorEvent::Commit(Advance::Stay));
                }
            }))
            .child(div().flex_1().min_w_0().child(self.accessible.clone()))
    }
}

impl TableChanges {
    /// Page column `source` as a writable target name, or why it is not.
    fn editable_column(&self, source: usize) -> Result<String, SharedString> {
        let page = self
            .page
            .as_ref()
            .ok_or_else(|| SharedString::from("The table page is not loaded"))?;
        let column = page
            .columns
            .get(source)
            .ok_or_else(|| SharedString::from("The column is no longer on this page"))?;
        let analysis = self
            .analysis
            .as_ref()
            .ok_or_else(|| self.unavailable.clone().unwrap_or_else(|| CHECKING.into()))?;
        match analysis.columns.get(source).map(|column| column.writability) {
            Some(ColumnWritability::Writable) => Ok(column.name.clone()),
            Some(ColumnWritability::Generated) => Err("Generated column".into()),
            Some(ColumnWritability::IdentityAlways) => {
                Err("Identity column (GENERATED ALWAYS)".into())
            }
            Some(ColumnWritability::SystemColumn) => Err("System column".into()),
            None => Err("The column is no longer on this page".into()),
        }
    }
    fn row_has_identity(&self) -> bool {
        self.analysis
            .as_ref()
            .and_then(|analysis| analysis.tables.first())
            .is_some_and(|table| table.identity.kind != MutationIdentityKind::None)
    }
    /// Opens an editor for a page or insert-band cell. Errors are refusal
    /// reasons for the footer status; nothing changes when one is returned.
    pub fn begin_edit(
        &mut self,
        cell: CellRef,
        source: usize,
        seed: EditSeed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        if let Some(edit) = &self.edit {
            if edit.cell == Some((cell, source)) {
                window.focus(&edit.editor.focus_handle(cx), cx);
                return Ok(());
            }
            return Err("Save or cancel the open edit first".into());
        }
        self.can_edit_now()?;
        if self.is_query() {
            return Err("Edit query results from the result toolbar".into());
        }
        let name = self.editable_column(source)?;
        let page = self.page.clone().expect("checked by editable_column");
        let mut insert_default = false;
        let value: Option<String> = match cell {
            CellRef::Page(row) => {
                if !self.row_has_identity() {
                    return Err(NO_IDENTITY.into());
                }
                if matches!(self.overlay().mark(row), Some(RowMark::Deleted { .. })) {
                    return Err("The row is staged for deletion. Revert it to edit".into());
                }
                let values = page
                    .rows
                    .get(row)
                    .ok_or_else(|| SharedString::from("The row is no longer on this page"))?;
                let hidden = page
                    .row_identity
                    .as_ref()
                    .and_then(|rows| rows.get(row))
                    .map(Vec::as_slice);
                self.draft
                    .as_ref()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| {
                        draft.edit_value(0, values, hidden, page.truncated_cells > 0, source)
                    })
                    .map_err(|error| refusal("Cell editing", error))?
                    .clone()
            }
            CellRef::Insert(id) => {
                let value = self
                    .draft
                    .as_ref()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| draft.insert_value(id, &name))
                    .map_err(|error| refusal("Cell editing", error))?;
                insert_default = value.is_none();
                value.flatten().map(str::to_owned)
            }
        };
        if value
            .as_ref()
            .is_some_and(|value| value.len() > cell_value::MAX_VALUE_BYTES)
        {
            return Err("The cell exceeds the 1 MiB editor limit; the value was kept".into());
        }
        let kind = page
            .columns
            .get(source)
            .and_then(|column| cell_value::classify(Some(&column.cast_type)));
        let (presentation, seeded) = edit_presentation(kind, value.as_deref(), &seed);
        let default = insert_default && seed == EditSeed::Keep;
        if !self.admit_work() {
            return Err(self.message.clone().into());
        }
        let null = seeded.is_none();
        let row = match cell {
            CellRef::Page(row) => Some(row),
            CellRef::Insert(_) => None,
        };
        self.open_edit(
            (row, Some(name), 0),
            seeded.unwrap_or_default(),
            null,
            presentation,
            window,
            cx,
        );
        let Some(edit) = self.edit.as_mut() else {
            self.finish_work();
            return Err("The editor could not open".into());
        };
        edit.cell = Some((cell, source));
        edit.insert = match cell {
            CellRef::Insert(id) => Some(id),
            CellRef::Page(_) => None,
        };
        edit.default = default;
        if presentation == Presentation::Inline {
            let editor = edit.editor.clone();
            let accessible = edit.accessible.clone();
            let placeholder = if default {
                Some("DEFAULT")
            } else if null {
                Some("NULL")
            } else {
                None
            };
            if let Some(placeholder) = placeholder {
                editor.update(cx, |editor, cx| {
                    editor.set_placeholder_text(placeholder, window, cx)
                });
            }
            editor.update(cx, |editor, cx| {
                editor.move_to_end(&editor::actions::MoveToEnd, window, cx)
            });
            self.install_cell_editor(editor, accessible, window, cx);
        }
        self.message.clear();
        cx.emit(ChangesEvent::EditOpened {
            cell,
            source,
            popover: presentation == Presentation::Popover,
        });
        cx.notify();
        Ok(())
    }
    fn install_cell_editor(
        &mut self,
        editor: Entity<Editor>,
        accessible: Entity<AccessibleEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|_| CellEditor::new(editor, accessible));
        let events = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &CellEditorEvent, window, cx| match *event {
                CellEditorEvent::Commit(advance) => {
                    if !this.pending() {
                        this.stage(advance, window, cx);
                    }
                }
                CellEditorEvent::Cancel => this.activate(Action::CancelEdit, window, cx),
                CellEditorEvent::Expand => this.expand(window, cx),
            },
        );
        self.cell_editor = Some((view, events));
    }
    /// The literal guard swapped the editor entity; keep the grid's view on it.
    pub(super) fn sync_cell_editor(&mut self, cx: &mut Context<Self>) {
        let (Some((view, _)), Some(edit)) = (&self.cell_editor, &self.edit) else {
            return;
        };
        let (editor, accessible) = (edit.editor.clone(), edit.accessible.clone());
        view.update(cx, |view, cx| view.replace(editor, accessible, cx));
    }
    /// `alt-enter`: move the draft text into the popover editor.
    fn expand(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = &self.edit else {
            return;
        };
        if edit.presentation == Presentation::Popover {
            return;
        }
        let text = edit.editor.read(cx).text(cx);
        let label = format!("Value for {}", edit.column.as_deref().unwrap_or("cell"));
        let editor = cx.new(|cx| {
            Editor::for_buffer(
                cx.new(|cx| language::Buffer::local(text.clone(), cx)),
                None,
                window,
                cx,
            )
        });
        let accessible = cx.new(|cx| AccessibleEditor::new(editor.clone(), label, cx));
        let edit = self.edit.as_mut().unwrap();
        edit.editor = editor.clone();
        edit.accessible = accessible;
        edit.history = super::literal_guard::History::new(text);
        edit.presentation = Presentation::Popover;
        let cell = edit.cell;
        self.cell_editor = None;
        window.focus(&editor.focus_handle(cx), cx);
        self.install_literal_guard(window, cx);
        if let Some((cell, source)) = cell {
            cx.emit(ChangesEvent::EditOpened {
                cell,
                source,
                popover: true,
            });
        }
        cx.notify();
    }
    /// Stages NULL for a cell without opening an editor.
    pub fn set_null(
        &mut self,
        cell: CellRef,
        source: usize,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        if self.edit.is_some() {
            return Err("Save or cancel the open edit first".into());
        }
        self.can_edit_now()?;
        let name = self.editable_column(source)?;
        if !self.admit_work() {
            return Err(self.message.clone().into());
        }
        let result = match cell {
            CellRef::Page(row) => self.stage_page_value(row, name, None),
            CellRef::Insert(id) => self
                .draft
                .as_mut()
                .ok_or(ModelError::Unavailable)
                .and_then(|draft| draft.set_insert_value(id, &name, Some(None))),
        };
        self.finish_change("Set NULL", result, cx)
    }
    /// Reverts one staged cell (`Some(source)`) or the whole staged row or
    /// insert (`None`). Removing a staged change needs no fresh analysis.
    pub fn revert(
        &mut self,
        cell: CellRef,
        source: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        self.can_select_now()?;
        let result = match (cell, source) {
            (CellRef::Insert(id), None) => self
                .draft
                .as_mut()
                .ok_or(ModelError::Unavailable)
                .and_then(|draft| draft.remove(id)),
            (CellRef::Insert(id), Some(source)) => {
                let name = self.editable_column(source)?;
                self.draft
                    .as_mut()
                    .ok_or(ModelError::Unavailable)
                    .and_then(|draft| draft.set_insert_value(id, &name, None))
            }
            (CellRef::Page(row), source) => {
                let overlay = self.overlay();
                match (overlay.mark(row), source) {
                    (Some(RowMark::Deleted { change, .. }), _)
                    | (Some(RowMark::Updated { change, .. }), None) => {
                        let change = *change;
                        self.draft
                            .as_mut()
                            .ok_or(ModelError::Unavailable)
                            .and_then(|draft| draft.remove(change))
                    }
                    (Some(RowMark::Updated { .. }), Some(source)) => {
                        if overlay.cell(row, source).is_none() {
                            return Err("Nothing is staged in this cell".into());
                        }
                        self.can_edit_now()?;
                        let name = self.editable_column(source)?;
                        // Staging the original value drops the column from the
                        // update, and the whole update when nothing is left.
                        let original = self
                            .page
                            .as_ref()
                            .and_then(|page| page.rows.get(row))
                            .and_then(|values| values.get(source))
                            .cloned()
                            .ok_or_else(|| {
                                SharedString::from("The row is no longer on this page")
                            })?;
                        if !self.admit_work() {
                            return Err(self.message.clone().into());
                        }
                        self.stage_page_value(row, name, original)
                    }
                    (None, _) => return Err("Nothing is staged in this row".into()),
                }
            }
        };
        self.finish_change("Revert", result, cx)
    }
    fn stage_page_value(
        &mut self,
        row: usize,
        column: String,
        value: Option<String>,
    ) -> Result<(), ModelError> {
        let page = self.page.clone().ok_or(ModelError::Unavailable)?;
        let values = page.rows.get(row).ok_or(ModelError::InvalidInput)?;
        let hidden = page
            .row_identity
            .as_ref()
            .and_then(|rows| rows.get(row))
            .map(Vec::as_slice);
        self.draft
            .as_mut()
            .ok_or(ModelError::Unavailable)
            .and_then(|draft| {
                draft.stage_update(
                    0,
                    values,
                    hidden,
                    page.truncated_cells > 0,
                    vec![MutationValue { column, value }],
                )
            })
    }
    /// Shared tail for direct (editor-less) draft operations.
    pub(super) fn finish_change(
        &mut self,
        action: &str,
        result: Result<(), ModelError>,
        cx: &mut Context<Self>,
    ) -> Result<(), SharedString> {
        let refused = result.as_ref().err().map(|error| refusal(action, *error));
        self.changed(result, cx);
        match refused {
            Some(reason) => {
                self.message = reason.to_string();
                Err(reason)
            }
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_follows_type_shape_and_size() {
        let json = cell_value::classify(Some("jsonb"));
        assert!(json.is_some());
        assert_eq!(
            edit_presentation(json, Some("{}"), &EditSeed::Keep),
            (Presentation::Popover, Some("{}".to_owned()))
        );
        assert_eq!(
            edit_presentation(None, Some("first\nsecond"), &EditSeed::Keep).0,
            Presentation::Popover
        );
        let long = "x".repeat(5 * 1024);
        assert_eq!(
            edit_presentation(None, Some(&long), &EditSeed::Keep).0,
            Presentation::Popover
        );
        assert_eq!(
            edit_presentation(None, Some("42"), &EditSeed::Keep),
            (Presentation::Inline, Some("42".to_owned()))
        );
        // Typing into a JSON cell still opens the popover, seeded with the key.
        assert_eq!(
            edit_presentation(json, Some("{\"a\":1}"), &EditSeed::Replace("[".into())),
            (Presentation::Popover, Some("[".to_owned()))
        );
    }

    #[test]
    fn seeds_keep_null_clear_to_empty_and_replace_multiline_values() {
        assert_eq!(
            edit_presentation(None, None, &EditSeed::Keep),
            (Presentation::Inline, None)
        );
        assert_eq!(
            edit_presentation(None, None, &EditSeed::Clear),
            (Presentation::Inline, Some(String::new()))
        );
        // A replacement discards the multi-line original, so it stays inline.
        assert_eq!(
            edit_presentation(None, Some("a\nb"), &EditSeed::Replace("z".into())),
            (Presentation::Inline, Some("z".to_owned()))
        );
    }

    #[test]
    fn inline_keys_commit_move_expand_and_cancel() {
        let key = |text: &str| cell_editor_key(&Keystroke::parse(text).unwrap());
        assert_eq!(key("enter"), Some(CellEditorEvent::Commit(Advance::Stay)));
        assert_eq!(
            key("cmd-enter"),
            Some(CellEditorEvent::Commit(Advance::Stay))
        );
        assert_eq!(key("tab"), Some(CellEditorEvent::Commit(Advance::Right)));
        assert_eq!(
            key("shift-tab"),
            Some(CellEditorEvent::Commit(Advance::Left))
        );
        assert_eq!(key("alt-enter"), Some(CellEditorEvent::Expand));
        assert_eq!(key("escape"), Some(CellEditorEvent::Cancel));
        assert_eq!(key("ctrl-enter"), None);
        assert_eq!(key("shift-enter"), None);
        assert_eq!(key("a"), None);
    }
}

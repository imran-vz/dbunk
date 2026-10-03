//! Explicit retained-result export. The immutable selection and all derived
//! buffers reserve shared retention until the joined job has finished.
use crate::{
    accessible_editor::AccessibleEditor,
    controller::Host,
    result_export::{self, Completeness, Encoding, ExportTable, Format, Options, Scope, SqlTarget},
    results::encoded_size,
};
use dbunk_lib::backend::result_files as files;
use editor::{Editor, EditorEvent, EditorMode};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    Subscription, Window, div, prelude::*, rgb,
};
use multi_buffer::MultiBufferOffset;
use std::{cell::Cell, rc::Rc, sync::Arc};
const RESERVATION: usize = 36 * 1024 * 1024;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;
const FIELD_BYTES: usize = 8192;
const HISTORY_REFRESH: usize = 512 * 1024;
const HISTORY_LIMIT: usize = 1024 * 1024;
const FIELD_LABELS: [&str; 2] = ["Export NULL token", "SQL export table name"];

#[derive(Debug, PartialEq, Eq)]
enum FieldChange {
    Keep,
    Refresh,
    Refuse,
}
struct FieldHistory {
    committed: String,
    previous_len: usize,
    cost: usize,
}
impl FieldHistory {
    fn new(value: String) -> Self {
        Self {
            previous_len: value.len(),
            committed: value,
            cost: 0,
        }
    }
    fn change(&mut self, length: usize, marked: bool, edited: bool) -> FieldChange {
        if edited {
            // Full old/new lengths overcount replaced text, including deletions
            // retained by the CRDT. Charge even empty edits for operation history.
            self.cost = self
                .cost
                .saturating_add(self.previous_len)
                .saturating_add(length)
                .saturating_add(256);
            self.previous_len = length;
        }
        if length > FIELD_BYTES || (marked && self.cost >= HISTORY_LIMIT) {
            FieldChange::Refuse
        } else if !marked && self.cost >= HISTORY_REFRESH {
            FieldChange::Refresh
        } else {
            FieldChange::Keep
        }
    }
}

fn field_editor(
    value: &str,
    index: usize,
    read_only: bool,
    window: &mut Window,
    cx: &mut Context<ExportView>,
) -> (Entity<Editor>, Entity<AccessibleEditor>) {
    let editor = cx.new(|cx| {
        let buffer = cx.new(|cx| language::Buffer::local(value, cx));
        let mut editor = Editor::for_buffer(buffer, None, window, cx);
        editor.set_mode(EditorMode::SingleLine);
        editor.set_read_only(read_only);
        editor
    });
    let accessible =
        cx.new(|cx| AccessibleEditor::field(editor.clone(), FIELD_LABELS[index], false, cx));
    (editor, accessible)
}
struct Reservation {
    budget: Rc<Cell<usize>>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(RESERVATION));
    }
}
struct Data {
    columns: Vec<String>,
    rows: Vec<Vec<Option<String>>>,
    partial: bool,
    target: Option<(String, String)>,
}
pub struct Capture {
    data: Arc<Data>,
    lease: Rc<Reservation>,
}
impl Capture {
    pub fn new(
        table: &ExportTable<'_>,
        target: Option<(String, String)>,
        budget: Rc<Cell<usize>>,
    ) -> Result<Self, String> {
        if target
            .as_ref()
            .is_some_and(|(_, name)| name.len() > FIELD_BYTES)
        {
            return Err("SQL export table name exceeds 8 KiB".into());
        }
        if table.columns.is_empty()
            || table.columns.len() > 1024
            || table.rows.len() > 100_000
            || table.columns.len().saturating_mul(table.rows.len() + 1) > 100_000
        {
            return Err("Export snapshot exceeds 100,000 cells including headings".into());
        }
        let indexes = table.source_columns.map_or_else(
            || (0..table.columns.len()).collect::<Vec<_>>(),
            <[usize]>::to_vec,
        );
        if indexes.len() != table.columns.len() {
            return Err("Export column projection is stale".into());
        }
        let mut bytes = encoded_size(&(table.columns, &target));
        for row in table.rows {
            for &index in &indexes {
                bytes = bytes.saturating_add(encoded_size(
                    row.get(index).ok_or("Export row projection is stale")?,
                ));
            }
            if bytes > result_export::MAX_EXPORT_BYTES {
                return Err("Export snapshot exceeds 8 MiB".into());
            }
        }
        if bytes > result_export::MAX_EXPORT_BYTES
            || RESERVATION > WORKSPACE_BYTES.saturating_sub(budget.get())
        {
            return Err(
                "Export needs 36 MiB of shared retention; clear results or close another tool"
                    .into(),
            );
        }
        budget.set(budget.get() + RESERVATION);
        let lease = Rc::new(Reservation { budget });
        let data = Arc::new(Data {
            columns: table.columns.iter().map(|s| (*s).to_owned()).collect(),
            rows: table
                .rows
                .iter()
                .map(|row| indexes.iter().map(|&i| row[i].clone()).collect())
                .collect(),
            partial: table.completeness == Completeness::Partial,
            target,
        });
        Ok(Self { data, lease })
    }
}
const FORMATS: [Option<Format>; 8] = [
    Some(Format::Csv),
    Some(Format::Json),
    Some(Format::Sql),
    Some(Format::Html),
    Some(Format::Markdown),
    Some(Format::Txt),
    Some(Format::Tsv),
    None,
];
const EXTENSIONS: [&str; 8] = ["csv", "json", "sql", "html", "md", "txt", "tsv", "xlsx"];
#[derive(Clone, Copy)]
enum Action {
    Format,
    Encoding,
    Compression,
    Save,
    Cancel,
    Close,
}
pub struct Close;
pub struct ExportView {
    capture: Capture,
    host: Arc<Host>,
    format: usize,
    encoding: Encoding,
    gzip: bool,
    null: Entity<Editor>,
    null_accessible: Entity<AccessibleEditor>,
    target: Entity<Editor>,
    target_accessible: Entity<AccessibleEditor>,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
    busy: bool,
    cancellation: Option<files::Cancellation>,
    status: String,
    field_history: [FieldHistory; 2],
    field_events: Vec<Subscription>,
    field_notice: Option<String>,
}
impl EventEmitter<Close> for ExportView {}
impl ExportView {
    pub fn new(
        capture: Capture,
        host: Arc<Host>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let target_value = capture
            .data
            .target
            .as_ref()
            .map_or("table_name", |(_, t)| t)
            .to_owned();
        let (null, null_accessible) = field_editor("NULL", 0, false, window, cx);
        let (target, target_accessible) = field_editor(&target_value, 1, false, window, cx);
        let status = format!(
            "{} captured rows. Retained values only{}. NULL token may equal real text; XLSX represents empty text as a blank cell.",
            capture.data.rows.len(),
            if capture.data.partial {
                "; source is partial"
            } else {
                ""
            }
        );
        let mut view = Self {
            capture,
            host,
            format: 0,
            encoding: Encoding::Utf8,
            gzip: false,
            null,
            null_accessible,
            target,
            target_accessible,
            root: cx.focus_handle(),
            buttons: (0..6).map(|_| cx.focus_handle()).collect(),
            busy: false,
            cancellation: None,
            status,
            field_history: [
                FieldHistory::new("NULL".into()),
                FieldHistory::new(target_value),
            ],
            field_events: vec![],
            field_notice: None,
        };
        for index in 0..2 {
            let subscription = view.subscribe_field(index, window, cx);
            view.field_events.push(subscription);
        }
        view
    }
    fn field(&self, index: usize) -> &Entity<Editor> {
        if index == 0 { &self.null } else { &self.target }
    }
    fn subscribe_field(
        &self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            self.field(index),
            window,
            move |this, editor, event, window, cx| {
                if editor != this.field(index) {
                    return;
                }
                let edited = matches!(event, EditorEvent::BufferEdited);
                if edited
                    || matches!(
                        event,
                        EditorEvent::InputHandled { .. }
                            | EditorEvent::SelectionsChanged { .. }
                            | EditorEvent::Blurred
                    )
                {
                    this.check_field(index, edited, window, cx);
                }
            },
        )
    }
    fn check_field(
        &mut self,
        index: usize,
        edited: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = self.field(index).clone();
        let length = editor.read(cx).buffer().read(cx).len(cx).0;
        let marked = editor.update(cx, |editor, cx| {
            editor.marked_text_range(window, cx).is_some()
        });
        let change = self.field_history[index].change(length, marked, edited);
        if change == FieldChange::Keep {
            if !marked {
                self.field_history[index].committed = editor.read(cx).text(cx);
            }
            return;
        }
        let refused = change == FieldChange::Refuse;
        let value = if refused {
            self.field_history[index].committed.clone()
        } else {
            editor.read(cx).text(cx)
        };
        let focus = editor.focus_handle(cx).is_focused(window);
        let selection = editor.update(cx, |editor, cx| {
            let snapshot = editor.display_snapshot(cx);
            editor.selections.newest::<MultiBufferOffset>(&snapshot)
        });
        let (anchor, head) = if refused {
            (value.len(), value.len())
        } else if selection.reversed {
            (selection.end.0, selection.start.0)
        } else {
            (selection.start.0, selection.end.0)
        };
        // Zed's forget_transaction only drops undo entries; a fresh buffer also
        // releases deleted CRDT text. This happens at a threshold, not each edit.
        let (replacement, accessible) = field_editor(&value, index, self.busy, window, cx);
        replacement.update(cx, |editor, cx| {
            editor.change_selections(
                editor::SelectionEffects::no_scroll(),
                window,
                cx,
                |selections| {
                    selections.select_ranges([MultiBufferOffset(anchor)..MultiBufferOffset(head)]);
                },
            )
        });
        if focus {
            window.focus(&replacement.focus_handle(cx), cx);
        }
        if index == 0 {
            self.null = replacement;
            self.null_accessible = accessible;
        } else {
            self.target = replacement;
            self.target_accessible = accessible;
        }
        self.field_history[index] = FieldHistory::new(value);
        // Defer replacing the subscription until this listener has returned.
        cx.defer_in(window, move |this, window, cx| {
            this.field_events[index] = this.subscribe_field(index, window, cx);
        });
        self.field_notice = Some(format!(
            "{}: {}",
            FIELD_LABELS[index],
            if refused {
                "edit refused at the 8 KiB text or 1 MiB history limit; restored the last committed value and cleared undo history"
            } else {
                "undo history cleared at its size limit; current text and selection preserved"
            }
        ));
        cx.notify();
    }
    pub fn focus(&self) -> FocusHandle {
        self.buttons[0].clone()
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Close => true,
            Action::Cancel => self.busy,
            Action::Encoding => !self.busy && FORMATS[self.format].is_some(),
            _ => !self.busy,
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        match action {
            Action::Format => self.format = (self.format + 1) % FORMATS.len(),
            Action::Encoding => {
                self.encoding = if self.encoding == Encoding::Utf8 {
                    Encoding::Utf16Le
                } else {
                    Encoding::Utf8
                }
            }
            Action::Compression => self.gzip = !self.gzip,
            Action::Close => {
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                cx.emit(Close);
            }
            Action::Cancel => {
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                self.status="Cancellation requested; a file already admitted for publication may still finish".into();
            }
            Action::Save => self.save(window, cx),
        }
        cx.notify();
    }
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.null.read(cx).buffer().read(cx).len(cx).0 > FIELD_BYTES
            || self.target.read(cx).buffer().read(cx).len(cx).0 > FIELD_BYTES
        {
            self.status = "Export field exceeds 8 KiB".into();
            return;
        }
        let null = self.null.read(cx).text(cx);
        let target = self.target.read(cx).text(cx);
        let format = self.format;
        let encoding = self.encoding;
        let compression = if self.gzip {
            files::Compression::Gzip
        } else {
            files::Compression::None
        };
        let name = format!(
            "result.{}{}",
            EXTENSIONS[format],
            if self.gzip { ".gz" } else { "" }
        );
        let picker = cx.prompt_for_new_path(std::path::Path::new("/tmp"), Some(&name));
        let token = files::Cancellation::default();
        self.cancellation = Some(token.clone());
        self.busy = true;
        self.null.update(cx, |e, _| e.set_read_only(true));
        self.target.update(cx, |e, _| e.set_read_only(true));
        let host = self.host.clone();
        let data = self.capture.data.clone();
        let lease = self.capture.lease.clone();
        // Detached UI waiter retains the payload allowance even if this view is
        // closed. Host owns and joins the actual blocking job during shutdown.
        cx.spawn_in(window, async move |this, cx| {
            let result = match picker.await {
                Ok(Ok(Some(path))) if !token.is_cancelled() => {
                    let operation = move |cancel: &files::Cancellation| {
                        let headings = data.columns.iter().map(String::as_str).collect::<Vec<_>>();
                        let rows = data.rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
                        let source = files::Source {
                            completeness: if data.partial {
                                files::Completeness::Partial
                            } else {
                                files::Completeness::Complete
                            },
                            scope: files::Scope::RetainedRows,
                            row_count: data.rows.len(),
                        };
                        let prepared = if let Some(format) = FORMATS[format] {
                            let table = ExportTable {
                                columns: &headings,
                                rows: &rows,
                                source_columns: None,
                                completeness: if data.partial {
                                    Completeness::Partial
                                } else {
                                    Completeness::Complete
                                },
                            };
                            let mut options = Options::new(format);
                            options.encoding = encoding;
                            options.scope = Scope::RetainedRows;
                            options.null_as = &null;
                            options.sql_target = Some(SqlTarget {
                                schema: data.target.as_ref().map(|(s, _)| s.as_str()),
                                table: &target,
                            });
                            let export = result_export::export(&table, &options)
                                .map_err(|e| e.to_string())?;
                            files::prepare_bytes(export.bytes, compression, source, cancel)
                        } else {
                            files::prepare_xlsx(
                                &files::XlsxTable {
                                    columns: &headings,
                                    rows: &rows,
                                    source_columns: None,
                                    sheet_name: "Export",
                                    null_as: &null,
                                    source,
                                },
                                compression,
                                cancel,
                            )
                        }
                        .map_err(|e| e.to_string())?;
                        files::publish_new(prepared, &path, cancel).map_err(|e| e.to_string())
                    };
                    match host.files.start(&host.runtime, token.clone(), operation) {
                        Ok(job) => job.finish().await.map(|published| {
                            format!(
                                "Saved {} bytes to {}. Captured retained rows only{}.",
                                published.bytes,
                                published.path.display(),
                                if published.source.completeness == files::Completeness::Partial {
                                    "; partial source"
                                } else {
                                    ""
                                }
                            )
                        }),
                        Err(error) => Err(error.to_owned()),
                    }
                }
                Ok(Ok(_)) => Ok("Export cancelled".into()),
                _ => Err("Export picker failed".into()),
            };
            drop(lease);
            this.update_in(cx, |this, _, cx| {
                this.busy = false;
                this.cancellation = None;
                this.null.update(cx, |e, _| e.set_read_only(false));
                this.target.update(cx, |e, _| e.set_read_only(false));
                this.status = result.unwrap_or_else(|e| format!("Export refused: {e}"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    fn button(
        &self,
        index: usize,
        label: String,
        action: Action,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let enabled = self.enabled(action);
        let weak = cx.entity().downgrade();
        div()
            .id(("export-action", index))
            .role(Role::Button)
            .aria_label(label.clone())
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .track_focus(&self.buttons[index])
            .tab_stop(enabled)
            .tab_index(0)
            .focus(|s| s.bg(rgb(0x222222)))
            .px_2()
            .py_1()
            .border_1()
            .border_color(rgb(0x444444))
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
}
impl Drop for ExportView {
    fn drop(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
    }
}
impl Render for ExportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("retained-export")
            .role(Role::Group)
            .aria_label("Export captured retained rows")
            .track_focus(&self.root)
            .flex()
            .flex_col()
            .size_full()
            .bg(rgb(0))
            .text_color(rgb(0xffffff))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                for editor in [&this.null, &this.target] {
                    if editor.focus_handle(cx).is_focused(window)
                        && editor
                            .update(cx, |e, cx| e.marked_text_range(window, cx))
                            .is_some()
                    {
                        return;
                    }
                }
                let m = &event.keystroke.modifiers;
                if m.control || m.alt || m.platform {
                    return;
                }
                if event.keystroke.key == "escape" {
                    this.activate(Action::Close, window, cx);
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key != "tab" {
                    return;
                }
                let handles = this
                    .buttons
                    .iter()
                    .zip([
                        Action::Format,
                        Action::Encoding,
                        Action::Compression,
                        Action::Save,
                        Action::Cancel,
                        Action::Close,
                    ])
                    .filter(|(_, a)| this.enabled(*a))
                    .map(|(h, _)| h.clone())
                    .chain([this.null.focus_handle(cx), this.target.focus_handle(cx)])
                    .collect::<Vec<_>>();
                let current = handles.iter().position(|h| h.is_focused(window));
                let next = if m.shift {
                    current.map_or(handles.len() - 1, |i| {
                        (i + handles.len() - 1) % handles.len()
                    })
                } else {
                    current.map_or(0, |i| (i + 1) % handles.len())
                };
                window.focus(&handles[next], cx);
                cx.stop_propagation();
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(self.button(
                        0,
                        format!("Format: {}", EXTENSIONS[self.format]),
                        Action::Format,
                        cx,
                    ))
                    .child(self.button(
                        1,
                        format!(
                            "Encoding: {}",
                            if self.encoding == Encoding::Utf8 {
                                "UTF-8"
                            } else {
                                "UTF-16LE"
                            }
                        ),
                        Action::Encoding,
                        cx,
                    ))
                    .child(self.button(
                        2,
                        format!("Compression: {}", if self.gzip { "gzip" } else { "none" }),
                        Action::Compression,
                        cx,
                    ))
                    .child(self.button(3, "Save new file".into(), Action::Save, cx))
                    .child(self.button(4, "Cancel export".into(), Action::Cancel, cx))
                    .child(self.button(5, "Results".into(), Action::Close, cx)),
            )
            .child(
                div()
                    .flex()
                    .h_8()
                    .child("NULL token")
                    .child(div().flex_1().child(self.null_accessible.clone())),
            )
            .child(
                div()
                    .flex()
                    .h_8()
                    .child("SQL table name")
                    .child(div().flex_1().child(self.target_accessible.clone())),
            )
            .child(
                div()
                    .id("export-status")
                    .role(Role::Label)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
            .children(self.field_notice.as_ref().map(|notice| {
                div()
                    .id("export-field-notice")
                    .role(Role::Label)
                    .aria_label(notice.clone())
                    .child(notice.clone())
            }))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn field_limits_refuse_without_changing_committed_text() {
        let mut history = FieldHistory::new("雪".into());
        assert_eq!(history.change(FIELD_BYTES, false, true), FieldChange::Keep);
        assert_eq!(
            history.change(FIELD_BYTES + 1, true, true),
            FieldChange::Refuse
        );
        assert_eq!(history.committed, "雪");
        let mut history = FieldHistory::new(String::new());
        history.cost = HISTORY_LIMIT - 256;
        assert_eq!(history.change(0, true, true), FieldChange::Refuse);
    }
    #[test]
    fn history_refresh_waits_for_composition_and_counts_deletions() {
        let mut history = FieldHistory::new("x".repeat(FIELD_BYTES));
        history.cost = HISTORY_REFRESH - FIELD_BYTES;
        assert_eq!(history.change(0, true, true), FieldChange::Keep);
        assert!(history.cost >= HISTORY_REFRESH);
        assert_eq!(history.change(0, false, false), FieldChange::Refresh);
        let fresh = FieldHistory::new("retained text".into());
        assert_eq!(fresh.cost, 0);
        assert_eq!(fresh.committed, "retained text");
    }
    #[test]
    fn snapshot_projects_exact_values_and_retains_budget_until_last_owner() {
        let budget = Rc::new(Cell::new(19));
        let row = vec![Some("hidden".into()), None, Some("雪'\\n".into())];
        let rows = [row.as_slice()];
        let table = ExportTable {
            columns: &["value", "nullable"],
            rows: &rows,
            source_columns: Some(&[2, 1]),
            completeness: Completeness::Partial,
        };
        let capture = Capture::new(&table, None, budget.clone()).unwrap();
        assert_eq!(capture.data.rows[0], vec![Some("雪'\\n".into()), None]);
        assert!(capture.data.partial);
        let lease = capture.lease.clone();
        drop(capture);
        assert_eq!(budget.get(), 19 + RESERVATION);
        drop(lease);
        assert_eq!(budget.get(), 19);
        budget.set(WORKSPACE_BYTES);
        assert!(Capture::new(&table, None, budget.clone()).is_err());
        assert_eq!(budget.get(), WORKSPACE_BYTES);
    }
}

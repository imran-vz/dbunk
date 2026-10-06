//! Options precede capture: scalable CSV uses the owned CSV transfer tool;
//! other formats save a complete immutable capture through joined FileRuntime.
use crate::{
    bounded_field::Field,
    controller::Host,
    whole_table_export_model::{self as model, Capture, FORMATS, Lease},
};
use dbunk_lib::backend::{
    export_configurations::{
        ExportCompression, ExportEncoding, ExportFormat, ExportOptions, ExportTarget,
    },
    result_files as files,
    table_export::{TableExportCapture, TableExportRequest},
};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, Subscription,
    Window, div, prelude::*,
};
use std::{cell::Cell, rc::Rc, sync::Arc};
mod configurations;
mod save;
gpui::actions!(whole_table_export, [NextControl, PreviousControl]);
pub enum WholeExportEvent {
    Back,
    Connect,
    Capture(TableExportRequest),
    Cancel,
    Csv(String),
    LoadConfigurations,
    SaveConfiguration(
        ExportOptions,
        dbunk_lib::backend::export_configurations::ExportConfigurationsRevision,
    ),
}
#[derive(Clone, Copy)]
enum Action {
    Back,
    Connect,
    Format,
    Encoding,
    Compression,
    Capture,
    Save,
    Cancel,
    Clear,
    LoadConfigurations,
    SaveConfiguration,
    LoadLatest,
}
const ACTIONS: [(Action, &str); 12] = [
    (Action::Back, "Table"),
    (Action::Connect, "Connect"),
    (Action::Format, "Format"),
    (Action::Encoding, "Encoding"),
    (Action::Compression, "Compression"),
    (Action::Capture, "Capture whole table"),
    (Action::Save, "Save captured table"),
    (Action::Cancel, "Cancel"),
    (Action::Clear, "Clear capture"),
    (Action::LoadConfigurations, "Load configurations"),
    (Action::SaveConfiguration, "Save configuration"),
    (Action::LoadLatest, "Load latest configuration"),
];
pub struct WholeTableExportView {
    host: Arc<Host>,
    connection: String,
    schema: String,
    table: String,
    budget: Rc<Cell<usize>>,
    _fields: Rc<Lease>,
    null: Entity<Field>,
    _null_events: Subscription,
    capture: Option<Capture>,
    configurations: Option<configurations::Configurations>,
    config_busy: bool,
    format: usize,
    encoding: ExportEncoding,
    compression: ExportCompression,
    ready: bool,
    reading: bool,
    can_cancel_read: bool,
    editable: bool,
    saving: bool,
    cancellation: Option<files::Cancellation>,
    status: String,
    root: FocusHandle,
    buttons: Vec<FocusHandle>,
}
impl EventEmitter<WholeExportEvent> for WholeTableExportView {}
impl WholeTableExportView {
    pub fn new(
        host: Arc<Host>,
        target: ExportTarget,
        budget: Rc<Cell<usize>>,
        lease: Rc<Lease>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let ExportTarget {
            connection_id: connection,
            schema,
            table,
        } = target;
        let null = cx.new(|cx| {
            Field::new(
                "Whole-table export NULL token",
                8192,
                false,
                String::new(),
                window,
                cx,
            )
        });
        let events = cx.subscribe(&null, |_, _, _: &crate::bounded_field::Changed, cx| {
            cx.notify()
        });
        Self{host,connection,schema,table,budget,_fields:lease,null,_null_events:events,capture:None,configurations:None,config_busy:false,format:0,encoding:ExportEncoding::Utf8,compression:ExportCompression::None,ready:false,reading:false,can_cancel_read:false,editable:true,saving:false,cancellation:None,status:"CSV uses the streaming transfer tool. Other formats require a complete bounded capture first.".into(),root:cx.focus_handle(),buttons:ACTIONS.iter().map(|_|cx.focus_handle()).collect()}
    }
    pub fn focus(&self) -> FocusHandle {
        self.buttons[2].clone()
    }
    pub fn contains_focus(&self, window: &Window, cx: &gpui::App) -> bool {
        self.root.contains_focused(window, cx)
    }
    pub fn sync(
        &mut self,
        ready: bool,
        reading: bool,
        can_cancel_read: bool,
        editable: bool,
        cx: &mut Context<Self>,
    ) {
        if (
            self.ready,
            self.reading,
            self.can_cancel_read,
            self.editable,
        ) == (ready, reading, can_cancel_read, editable)
        {
            return;
        }
        self.ready = ready;
        self.reading = reading;
        self.can_cancel_read = can_cancel_read;
        self.editable = editable;
        self.null.update(cx, |field, cx| {
            field.set_readonly(!editable || reading || self.saving, cx)
        });
        cx.notify();
    }
    pub fn receive(
        &mut self,
        source: TableExportCapture,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let capture = Capture::new(
            source,
            &self.connection,
            &self.request(),
            self.budget.clone(),
        )?;
        let d = capture.source.data();
        self.status = format!(
            "Complete capture: {} rows, {} columns. {} to {}. Includes committed rows visible to this role and row-security policy. Grid filter and column visibility settings and staged edits do not affect this capture.",
            d.rows.len(),
            d.columns.len(),
            d.captured_start,
            d.captured_end
        );
        self.capture = Some(capture);
        cx.notify();
        Ok(())
    }
    pub fn fail(&mut self, message: String, cx: &mut Context<Self>) {
        self.status = message;
        cx.notify();
    }
    fn request(&self) -> TableExportRequest {
        model::request(
            &self.schema,
            &self.table,
            self.capture.as_ref().map(|c| c.source.data().identity),
        )
    }
    fn raw_options(&self, cx: &gpui::App) -> Result<ExportOptions, &'static str> {
        let options = ExportOptions {
            format: FORMATS[self.format].0,
            encoding: self.encoding,
            compression: self.compression,
            null_token: self.null.read(cx).value(cx)?,
        };
        options
            .validate()
            .map_err(|_| "Export options exceed bounds")?;
        Ok(options)
    }
    fn options(&self, cx: &gpui::App) -> Result<ExportOptions, &'static str> {
        let options = self.raw_options(cx)?;
        model::route(&options)?;
        Ok(options)
    }
    fn enabled(&self, action: Action) -> bool {
        match action {
            Action::Back => true,
            Action::Cancel => self.can_cancel_read || self.saving || self.config_busy,
            Action::LoadConfigurations => self.editable && !self.config_busy,
            Action::SaveConfiguration => {
                self.editable && !self.config_busy && self.configurations.is_some()
            }
            Action::LoadLatest => {
                self.editable
                    && !self.config_busy
                    && !self.reading
                    && !self.saving
                    && self
                        .configurations
                        .as_ref()
                        .is_some_and(|c| c.data.latest(&self.target()).is_some())
            }
            Action::Encoding => {
                self.editable
                    && !self.reading
                    && !self.saving
                    && FORMATS[self.format].0 != ExportFormat::Xlsx
            }
            Action::Connect => self.editable && !self.reading && !self.saving && !self.ready,
            Action::Capture => {
                self.editable
                    && !self.reading
                    && !self.saving
                    && (FORMATS[self.format].0 == ExportFormat::Csv || self.ready)
            }
            Action::Save => {
                self.editable
                    && !self.reading
                    && !self.saving
                    && self.capture.is_some()
                    && FORMATS[self.format].0 != ExportFormat::Csv
            }
            Action::Clear => !self.reading && !self.saving && self.capture.is_some(),
            _ => self.editable && !self.reading && !self.saving,
        }
    }
    fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if !self.enabled(action) {
            return;
        }
        if self
            .null
            .update(cx, |field, cx| field.composing(window, cx))
        {
            self.status = "Finish NULL-token composition before changing export options".into();
            cx.notify();
            return;
        }
        match action {
            Action::LoadConfigurations => cx.emit(WholeExportEvent::LoadConfigurations),
            Action::SaveConfiguration => self.save_configuration(cx),
            Action::LoadLatest => self.load_latest(window, cx),
            Action::Back => {
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                cx.emit(WholeExportEvent::Back);
            }
            Action::Connect => cx.emit(WholeExportEvent::Connect),
            Action::Cancel => {
                if let Some(token) = &self.cancellation {
                    token.cancel();
                }
                if self.can_cancel_read || self.config_busy {
                    cx.emit(WholeExportEvent::Cancel);
                }
                self.status =
                    "Cancellation requested; publication already admitted may finish".into();
            }
            Action::Format => self.format = (self.format + 1) % FORMATS.len(),
            Action::Encoding => {
                self.encoding = if self.encoding == ExportEncoding::Utf8 {
                    ExportEncoding::Utf16Le
                } else {
                    ExportEncoding::Utf8
                }
            }
            Action::Compression => {
                self.compression = if self.compression == ExportCompression::None {
                    ExportCompression::Gzip
                } else {
                    ExportCompression::None
                }
            }
            Action::Clear => {
                self.capture = None;
                self.status =
                    "Capture cleared. The next capture resolves the table identity again.".into();
            }
            Action::Capture => match self.options(cx) {
                Ok(options) => {
                    if options.format == ExportFormat::Csv {
                        cx.emit(WholeExportEvent::Csv(options.null_token));
                    } else {
                        cx.emit(WholeExportEvent::Capture(self.request()));
                    }
                }
                Err(error) => self.status = error.into(),
            },
            Action::Save => match self.options(cx) {
                Ok(options) => self.save(options, window, cx),
                Err(error) => self.status = error.into(),
            },
        }
        cx.notify();
    }
    fn focus_control(&mut self, reverse: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .null
            .update(cx, |field, cx| field.composing(window, cx))
        {
            return;
        }
        let mut order = ACTIONS
            .iter()
            .enumerate()
            .filter(|(_, (action, _))| self.enabled(*action))
            .map(|(index, _)| self.buttons[index].clone())
            .collect::<Vec<_>>();
        order.push(self.null.focus_handle(cx));
        let at = order
            .iter()
            .position(|focus| focus.contains_focused(window, cx));
        let next = if reverse {
            at.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
        } else {
            at.map_or(0, |i| (i + 1) % order.len())
        };
        window.focus(&order[next], cx);
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
        {
            return;
        }
        if self
            .null
            .update(cx, |field, cx| field.composing(window, cx))
        {
            return;
        }
        match event.keystroke.key.as_str() {
            "tab" => self.focus_control(event.keystroke.modifiers.shift, window, cx),
            "escape" => self.activate(Action::Back, window, cx),
            _ => return,
        }
        window.prevent_default();
        cx.stop_propagation();
    }
    fn click_button(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let action = ACTIONS[index].0;
        if !self.enabled(action) {
            return;
        }
        // Keep an active composition with its editor; otherwise Return should
        // act on the button that was just clicked, including an AX click.
        if self
            .null
            .update(cx, |field, cx| field.composing(window, cx))
        {
            self.status = "Finish NULL-token composition before changing export options".into();
            cx.notify();
            return;
        }
        window.focus(&self.buttons[index], cx);
        self.activate(action, window, cx);
    }
    fn button(&self, index: usize, cx: &Context<Self>) -> gpui::AnyElement {
        let (action, name) = ACTIONS[index];
        let enabled = self.enabled(action);
        let label = match action {
            Action::Format => format!("Format: {}", FORMATS[self.format].1),
            Action::Encoding if FORMATS[self.format].0 == ExportFormat::Xlsx => {
                "Encoding: XLSX container".into()
            }
            Action::Encoding => format!("Encoding: {:?}", self.encoding),
            Action::Compression => format!("Compression: {:?}", self.compression),
            Action::Capture if FORMATS[self.format].0 == ExportFormat::Csv => {
                "Open CSV export".into()
            }
            _ => name.into(),
        };
        let weak = cx.weak_entity();
        let primary = matches!(action, Action::Save);
        crate::ui::tool_button(
            ("whole-export-action", index),
            label,
            None,
            enabled,
            primary,
        )
        .track_focus(&self.buttons[index])
        .tab_stop(enabled)
        .tab_index(0)
        // GPUI maps a focused Enter/Space pair to this click on key-up.
        // A separate key-down activation would run the action twice.
        .on_click(cx.listener(move |this, _, window, cx| this.click_button(index, window, cx)))
        .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
            weak.update(cx, |this, cx| this.click_button(index, window, cx))
                .ok();
        })
        .into_any_element()
    }
}
impl Drop for WholeTableExportView {
    fn drop(&mut self) {
        if let Some(token) = &self.cancellation {
            token.cancel();
        }
    }
}
impl Render for WholeTableExportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let target = format!("Whole table: {:?}.{:?}", self.schema, self.table);
        let scope_disclosure = "Whole committed table only, including partition rows. No grid filters, projections or staged edits. CSV streams through its own review; other formats refuse oversized captures or output instead of truncating.";
        let format_disclosure = "SQL, JSON and text values preserve captured server text. Dates and other output depend on the reader session. XLSX uses text cells; empty text appears blank. Existing files are never replaced.";
        div()
            .id("whole-table-export")
            .key_context("WholeTableExport")
            .role(Role::Group)
            .aria_label("Export whole committed table")
            .track_focus(&self.root)
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_size(gpui::px(crate::style::FONT))
            .on_action(cx.listener(|this, _: &NextControl, window, cx| {
                this.focus_control(false, window, cx);
                cx.stop_propagation();
            }))
            .on_action(cx.listener(|this, _: &PreviousControl, window, cx| {
                this.focus_control(true, window, cx);
                cx.stop_propagation();
            }))
            .capture_key_down(cx.listener(|this, event, window, cx| this.key(event, window, cx)))
            .child(
                crate::ui::toolbar()
                    .children((0..ACTIONS.len()).map(|index| self.button(index, cx))),
            )
            .child(
                div()
                    .id("whole-export-target")
                    .role(Role::Label)
                    .aria_label(target.clone())
                    .px_2()
                    .pt_1()
                    .font_family(crate::style::MONO)
                    .text_color(crate::style::dim())
                    .child(target),
            )
            .child(div().p_2().child(self.null.clone()))
            .child(
                div()
                    .id("whole-export-scope-disclosure")
                    .role(Role::Label)
                    .aria_label(scope_disclosure)
                    .px_2()
                    .text_color(crate::style::dim())
                    .child(scope_disclosure),
            )
            .child(
                div()
                    .id("whole-export-format-disclosure")
                    .role(Role::Label)
                    .aria_label(format_disclosure)
                    .px_2()
                    .py_1()
                    .text_color(crate::style::dim())
                    .child(format_disclosure),
            )
            .child(crate::ui::grow())
            .child(
                crate::ui::status_line()
                    .id("whole-export-status")
                    .text_color(crate::style::dim())
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .child(self.status.clone()),
            )
    }
}

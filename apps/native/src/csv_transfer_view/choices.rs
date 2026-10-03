use super::*;
use crate::catalog::{Catalog, Kind};
use dbunk_lib::backend::objects::{MAX_CATALOG_NODES, PgObjectKind};
pub(super) struct Choices {
    catalog: Catalog,
    visible: Vec<usize>,
    _lease: model::Lease,
}
#[derive(Default)]
pub(super) struct Metadata {
    pub controls: Option<TableControls>,
    pub receiver: Option<TableReceiver>,
    pub opening: bool,
    pub pending: Option<u64>,
    pub ready: bool,
    pub next: u64,
    pub revision: u64,
    pub filter_revision: u64,
    pub tables: bool,
    pub selected: Option<usize>,
    pub capture: Option<Choices>,
}
impl Metadata {
    pub fn busy(&self) -> bool {
        self.opening || self.pending.is_some()
    }
}
impl CsvTransferView {
    pub fn has_pending(&self) -> bool {
        self.metadata
            .receiver
            .as_ref()
            .is_some_and(TableReceiver::has_pending)
    }
    pub fn invalidate_after_restore(&mut self, cx: &mut Context<Self>) {
        self.stop_choices();
        self.changed(cx);
        self.status =
            "Database changed; inspect again. Setup text and accepted transfers remain".into();
        cx.notify();
    }
    pub(super) fn stop_choices(&mut self) {
        if let Some(controls) = self.metadata.controls.take() {
            controls.stop();
        }
        self.metadata.receiver = None;
        self.metadata.ready = false;
        self.metadata.opening = false;
        self.metadata.pending = None;
        self.metadata.capture = None;
        self.metadata.selected = None;
    }
    pub(super) fn load_choices(&mut self, cx: &mut Context<Self>) {
        if self.metadata.busy() {
            return;
        }
        if self.metadata.ready {
            self.request_choices();
            return;
        }
        let Some(connection) = self.connection.clone() else {
            return;
        };
        match self
            .host
            .open_table_document(self.id.clone(), connection, self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.metadata.controls = Some(controls);
                self.metadata.receiver = Some(receiver);
                self.metadata.opening = true;
                self.status = "Connecting owned CSV choice reader".into();
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    fn request_choices(&mut self) {
        let Some(id) = self.metadata.next.checked_add(1) else {
            self.status = "Choice identity exhausted; reopen setup".into();
            return;
        };
        self.metadata.next = id;
        if let Some(controls) = &self.metadata.controls {
            match controls.send(TableCommand::Catalog(id)) {
                Ok(()) => {
                    self.metadata.pending = Some(id);
                    self.status = "Loading CSV target choices".into();
                }
                Err(error) => self.status = error.into(),
            }
        }
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(envelope) = self
            .metadata
            .receiver
            .as_ref()
            .and_then(TableReceiver::try_recv)
        else {
            return false;
        };
        match envelope.into_message() {
            TableMessage::Opened => {
                self.metadata.opening = false;
                self.metadata.ready = true;
                self.request_choices();
            }
            TableMessage::Catalog(id, result) if self.metadata.pending == Some(id) => {
                self.metadata.pending = None;
                match result {
                    Ok(mut data) => {
                        let result = (|| {
                            let lease = model::Lease::inspection(self.budget.clone())?;
                            let count = data
                                .schemas
                                .iter()
                                .try_fold(0usize, |count, schema| {
                                    count.checked_add(1)?.checked_add(schema.tables.len())
                                })
                                .ok_or("CSV choices exceed bounds")?;
                            if count > MAX_CATALOG_NODES
                                || data.schemas.iter().any(|schema| {
                                    schema.name.len() > 63
                                        || schema.tables.iter().any(|table| table.name.len() > 63)
                                })
                            {
                                return Err("CSV choice identity exceeds bounds");
                            }
                            for schema in &mut data.schemas {
                                schema.views = vec![];
                                schema.materialized_views = vec![];
                                schema.foreign_tables = vec![];
                                schema.sequences = vec![];
                                schema.functions = vec![];
                                schema.procedures = vec![];
                                schema.aggregates = vec![];
                                schema.types = vec![];
                                schema.domains = vec![];
                                schema.extensions = vec![];
                                for table in &mut schema.tables {
                                    table.comment = None;
                                    table.identity_args = None;
                                    table.type_class = None;
                                }
                            }
                            data.roles = vec![];
                            data.tablespaces = vec![];
                            data.event_triggers = vec![];
                            data.truncated
                                .retain(|item| matches!(item.kind.as_str(), "schema" | "table"));
                            Ok(Choices {
                                catalog: Catalog::new(data, self.budget.clone())?,
                                visible: Vec::with_capacity(count),
                                _lease: lease,
                            })
                        })();
                        match result {
                            Ok(capture) => {
                                self.metadata.capture = Some(capture);
                                self.metadata.revision = id;
                                self.metadata.selected = None;
                                self.filter_choices();
                                self.status="Choices loaded. Exact names remain editable when choices are missing or truncated".into();
                            }
                            Err(error) => self.status = error.into(),
                        }
                    }
                    Err(error) => {
                        self.status =
                            format!("CSV choice read failed: {error:?}; previous choices retained")
                    }
                }
            }
            TableMessage::Catalog(..) => {}
            TableMessage::Error(error) => {
                self.status = format!("CSV choice reader failed: {error}")
            }
            TableMessage::Closed(result) => {
                self.metadata.controls = None;
                self.metadata.receiver = None;
                self.metadata.ready = false;
                self.metadata.opening = false;
                self.metadata.pending = None;
                self.status = match result {
                    Ok(_) => "CSV choice reader closed; Load choices reconnects explicitly".into(),
                    Err(error) => format!("CSV choice reader cleanup failed: {error}"),
                };
            }
            _ => self.status = "Unexpected CSV choice reader reply".into(),
        }
        cx.notify();
        true
    }
    pub(super) fn filter_choices(&mut self) {
        if let Some(capture) = &mut self.metadata.capture {
            capture.visible.clear();
            capture
                .visible
                .extend(
                    capture
                        .catalog
                        .rows
                        .iter()
                        .enumerate()
                        .filter_map(|(index, row)| {
                            (if self.metadata.tables {
                                matches!(row.kind, Kind::Object(PgObjectKind::Table))
                                    && row.schema.as_deref() == Some(self.schema_seen.as_str())
                            } else {
                                matches!(row.kind, Kind::Object(PgObjectKind::Schema))
                            })
                            .then_some(index)
                        }),
                );
        }
    }
    pub(super) fn use_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.setup_enabled(cx) || self.composing(window, cx) {
            return;
        }
        let Some(row) = self.metadata.selected.and_then(|index| {
            let capture = self.metadata.capture.as_ref()?;
            capture.catalog.rows.get(*capture.visible.get(index)?)
        }) else {
            return;
        };
        let name = row.entry.name.clone();
        let schema = row.schema.clone();
        if self.metadata.tables && schema.as_deref() != Some(self.schema_seen.as_str()) {
            return;
        }
        let index = usize::from(self.metadata.tables);
        let result = self
            .fields
            .get(index)
            .ok_or("CSV field allowance unavailable")
            .and_then(|field| field.update(cx, |field, cx| field.set_value(name, window, cx)));
        match result {
            Ok(()) => {
                self.changed(cx);
                if let Err(error) = self.reconcile_schema(window, cx) {
                    self.status = error.into();
                }
            }
            Err(error) => self.status = error.into(),
        }
        cx.notify();
    }
    pub(super) fn choices_element(&self, cx: &Context<Self>) -> AnyElement {
        let count = self
            .metadata
            .capture
            .as_ref()
            .map_or(0, |capture| capture.visible.len());
        let truncated = self
            .metadata
            .capture
            .as_ref()
            .is_some_and(|capture| !capture.catalog.truncated.is_empty());
        div().child(div().flex().flex_wrap().children((14..19).map(|index|self.button(index,cx))))
            .child("Ordinary and partitioned tables. Typing does not query PostgreSQL. Enter exact names when choices are missing.")
            .when(truncated,|body|body.child(div().id("csv-choice-limit").role(Role::Alert).child("Catalog choices are truncated; missing names are not proof of absence.")))
            .when(self.metadata.capture.is_some(),|body|body.child(div().id("csv-choice-list").role(Role::ListBox).aria_label("CSV schema and table choices").track_focus(&self.choice_focus).tab_index(0)
                .on_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{if !this.choice_focus.is_focused(window){return;}let count=this.metadata.capture.as_ref().map_or(0,|capture|capture.visible.len());match event.keystroke.key.as_str(){"up"=>this.metadata.selected=Some(this.metadata.selected.unwrap_or(0).saturating_sub(1)),"down"=>this.metadata.selected=Some(this.metadata.selected.map_or(0,|index|(index+1).min(count.saturating_sub(1)))),"home"=>this.metadata.selected=Some(0),"end"=>this.metadata.selected=count.checked_sub(1),"enter"=>{this.use_choice(window,cx);},_=>return}cx.notify();cx.stop_propagation();}))
                .child(gpui::uniform_list("csv-choices",count,cx.processor(|this,range:std::ops::Range<usize>,_,cx|{range.map(|index|{
                    let Some(row)=this.metadata.capture.as_ref().and_then(|capture|capture.visible.get(index).and_then(|row|capture.catalog.rows.get(*row)))else{return div().into_any_element();};
                    let label=row.entry.name.clone();let revision=this.metadata.revision;let filter=this.metadata.filter_revision;let selected=this.metadata.selected==Some(index);let weak=cx.weak_entity();
                    div().id(("csv-choice",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).h(px(28.)).px_2().when(selected,|row|row.bg(rgb(0x252525))).child(label)
                        .on_click(cx.listener(move|this,_,window,cx|{if revision==this.metadata.revision&&filter==this.metadata.filter_revision{this.metadata.selected=Some(index);window.focus(&this.choice_focus,cx);cx.notify();}}))
                        .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|{if revision==this.metadata.revision&&filter==this.metadata.filter_revision{this.metadata.selected=Some(index);window.focus(&this.choice_focus,cx);cx.notify();}}).ok();}).into_any_element()
                }).collect()})).h(px(112.)).track_scroll(&self.choice_scroll)))).into_any_element()
    }
}

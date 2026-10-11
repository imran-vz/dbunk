//! Plan 033 PostgreSQL structure draft. The editor's document-bound calls
//! ride this Objects document worker, and its draft and apply attempt are
//! journaled with this workspace document (`WorkspaceSchemaEdit`).
use super::*;
use crate::schema_edit_view::{Channel, SchemaEditEvent, SchemaEditView};
use dbunk_lib::backend::WorkspaceSchemaEdit;
use dbunk_lib::backend::schema_edit::SchemaEditRequest;

#[derive(Default)]
pub(super) struct SchemaEditLane {
    view: Option<Entity<SchemaEditView>>,
    /// The restored journal until an editor takes it over.
    recovery: Option<WorkspaceSchemaEdit>,
    events: Option<gpui::Subscription>,
    pub(super) busy: bool,
    show: bool,
}

impl SchemaEditLane {
    pub(super) fn new(recovery: Option<WorkspaceSchemaEdit>) -> Self {
        Self {
            recovery,
            ..Self::default()
        }
    }
}

impl CatalogView {
    pub fn schema_edit_snapshot(&self, cx: &gpui::App) -> Option<WorkspaceSchemaEdit> {
        self.schema_edit.view.as_ref().map_or_else(
            || self.schema_edit.recovery.clone(),
            |view| view.read(cx).snapshot(),
        )
    }

    pub fn schema_edit_bytes(&self, cx: &gpui::App) -> usize {
        self.schema_edit.view.as_ref().map_or_else(
            || {
                self.schema_edit
                    .recovery
                    .as_ref()
                    .map_or(0, crate::results::encoded_size)
            },
            |view| view.read(cx).snapshot_bytes(),
        )
    }

    pub fn has_schema_edit_changes(&self, cx: &gpui::App) -> bool {
        self.schema_edit.recovery.is_some()
            || self
                .schema_edit
                .view
                .as_ref()
                .is_some_and(|view| view.read(cx).has_changes())
    }

    pub(super) fn sync_schema_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.schema_edit.view {
            let controls = if self.ready && self.editable {
                self.controls.clone()
            } else {
                None
            };
            view.update(cx, |view, cx| view.set_runtime(controls, cx));
        }
    }

    /// The editor while it is shown in place of the structure.
    pub(super) fn schema_edit_shown(&self) -> Option<Entity<SchemaEditView>> {
        self.schema_edit
            .view
            .clone()
            .filter(|_| self.schema_edit.show)
    }

    pub(super) fn open_schema_edit(
        &mut self,
        schema: String,
        table: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let request = SchemaEditRequest::new(schema, table);
        if let Some(view) = &self.schema_edit.view
            && view.read(cx).request() != &request
        {
            if view.read(cx).has_changes() {
                let other = view.read(cx).request().clone();
                self.status = format!(
                    "A structure draft for {}.{} is open; apply or discard it first",
                    other.schema, other.table
                );
                cx.notify();
                return;
            }
            self.schema_edit.view = None;
            self.schema_edit.events = None;
        }
        if let Some(recovery) = &self.schema_edit.recovery
            && recovery.request != request
        {
            self.status = format!(
                "A saved structure draft for {}.{} is retained; open that table's structure to resume it",
                recovery.request.schema, recovery.request.table
            );
            cx.notify();
            return;
        }
        if let Some(view) = &self.schema_edit.view {
            view.update(cx, |view, cx| view.resume(cx));
        } else {
            if !self.ready || !self.editable || self.controls.is_none() {
                self.status = "Connect the Objects document before editing a structure".into();
                cx.notify();
                return;
            }
            let Some(connection) = self.connection.clone() else {
                self.status = "Select a connection".into();
                cx.notify();
                return;
            };
            let host = self.host.clone();
            let ids = self.apply_ids.clone();
            let controls = self.controls.clone();
            let restored = self.schema_edit.recovery.take();
            let view = cx.new(|cx| {
                SchemaEditView::new(
                    host,
                    Channel::Worker(controls),
                    connection,
                    request,
                    true,
                    ids,
                    restored,
                    false,
                    cx,
                )
            });
            self.schema_edit.events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &SchemaEditEvent, window, cx| {
                    match event {
                        SchemaEditEvent::Changed => cx.emit(CatalogEvent::Changed),
                        SchemaEditEvent::PersistApply(id) => {
                            cx.emit(CatalogEvent::PersistApply(*id))
                        }
                        SchemaEditEvent::Activity(busy) => this.schema_edit.busy = *busy,
                        SchemaEditEvent::Close => {
                            this.schema_edit.show = false;
                            if let Some(structure) = &this.structure {
                                window.focus(&structure.read(cx).focus(cx), cx);
                            }
                        }
                        SchemaEditEvent::StructureChanged => {
                            // Same as the table change flow: every document
                            // on this connection re-reads before reuse.
                            if let Some(connection) = this.connection.clone() {
                                cx.emit(CatalogEvent::DatabaseChanged(connection));
                            }
                        }
                    }
                    cx.notify();
                },
            ));
            // The first observation started before this subscription existed.
            self.schema_edit.busy = view.read(cx).has_pending();
            self.schema_edit.view = Some(view);
        }
        self.schema_edit.show = true;
        if let Some(view) = &self.schema_edit.view {
            window.focus(&view.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Another table's structure arrived: show it instead of the editor,
    /// which keeps its draft.
    pub(super) fn schema_edit_follow(&mut self, schema: &str, table: &str, cx: &gpui::App) {
        if self.schema_edit.view.as_ref().is_some_and(|view| {
            let request = view.read(cx).request();
            request.schema != schema || request.table != table
        }) {
            self.schema_edit.show = false;
        }
    }

    pub(super) fn route_schema_edit(&mut self, message: TableMessage, cx: &mut Context<Self>) {
        if let TableMessage::SchemaEdit(id, reply) = message
            && let Some(view) = &self.schema_edit.view
        {
            view.update(cx, |view, cx| view.receive(id, reply, cx));
        }
    }

    pub(super) fn schema_edit_disconnected(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &self.schema_edit.view {
            view.update(cx, |view, cx| view.disconnected(cx));
        }
        self.schema_edit.busy = false;
    }

    pub(super) fn schema_edit_saved(
        &mut self,
        id: u64,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.schema_edit.view {
            view.update(cx, |view, cx| view.apply_saved(id, result, cx));
        }
    }

    /// Surfaces a restored draft or unknown attempt that no editor holds yet.
    pub(super) fn schema_edit_banner(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        if self.schema_edit.view.is_some() {
            return None;
        }
        let recovery = self.schema_edit.recovery.as_ref()?;
        let unknown = recovery.outcome_unknown();
        let (schema, table) = (
            recovery.request.schema.clone(),
            recovery.request.table.clone(),
        );
        let text = if unknown {
            format!(
                "Structure change on {schema}.{table} has an unknown outcome; inspect and reconcile it"
            )
        } else {
            format!("Saved structure draft for {schema}.{table}")
        };
        let enabled = self.ready && self.editable && !self.has_pending();
        Some(
            div()
                .id("catalog-schema-edit-recovery")
                .role(Role::Alert)
                .aria_label(SharedString::from(text.clone()))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .bg(if unknown {
                    crate::style::bad_fill()
                } else {
                    crate::style::warn_fill()
                })
                .text_color(if unknown {
                    crate::style::bad_text()
                } else {
                    crate::style::warn()
                })
                .child(div().flex_1().min_w_0().child(text))
                .child(
                    crate::ui::button(
                        "catalog-schema-edit-open",
                        if unknown { "Reconcile" } else { "Resume" },
                        crate::ui::Variant::Secondary,
                        enabled,
                    )
                    .tab_index(0)
                    .when(enabled, |button| {
                        button.on_click(cx.listener(move |this, _, window, cx| {
                            this.open_schema_edit(schema.clone(), table.clone(), window, cx)
                        }))
                    }),
                )
                .into_any_element(),
        )
    }
}

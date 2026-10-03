use super::CatalogView;
use crate::controller::TableCommand;
use dbunk_lib::backend::objects::PgObjectRef;
use gpui::Context;

impl CatalogView {
    /// Navigator activation for a non-relation object: describe the exact
    /// overload-safe reference on this document's connection.
    pub fn describe_context(&mut self, reference: PgObjectRef, cx: &mut Context<Self>) {
        if self.ready {
            if self.busy || self.schema_busy || self.maintenance_busy || self.table_ddl_busy {
                self.status = "Finish the current Objects operation, then describe again".into();
            } else {
                self.request_description(reference, false);
            }
        } else if self.editable {
            self.structure_after_connect = None;
            self.describe_after_connect = Some(reference);
            if self.controls.is_none() {
                self.begin_connect(cx);
            }
        }
        cx.notify();
    }

    pub(super) fn request_description(&mut self, reference: PgObjectRef, impact: bool) {
        self.next = self.next.wrapping_add(1);
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| {
                controls.send(if impact {
                    TableCommand::DropImpact(self.next, reference)
                } else {
                    TableCommand::Describe(self.next, reference)
                })
            }) {
            Ok(()) => {
                self.pending = Some(self.next);
                self.cancellation_requested = false;
                self.busy = true;
                self.status = if impact {
                    "Reading downstream drop impact; no DDL is executed"
                } else {
                    "Loading object metadata"
                }
                .into();
            }
            Err(error) => self.status = error.into(),
        }
    }
}

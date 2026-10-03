//! Whole-export requests share the table's owned read/cancel lane, never its page.
use super::*;
use crate::{
    whole_table_export_model::{self as model, ReadState},
    whole_table_export_view::{WholeExportEvent, WholeTableExportView},
};
use dbunk_lib::backend::{
    export_configurations::ExportTarget,
    table_export::{TableExportCapture, TableExportRequest},
};
#[derive(Default)]
pub(super) struct WholeExport {
    pub view: Option<Entity<WholeTableExportView>>,
    pub read: ReadState,
    pub config: super::whole_config::ConfigLane,
    events: Option<Subscription>,
}
impl TableView {
    pub(super) fn open_whole_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        if self.whole_export.view.is_none() {
            let target = ExportTarget {
                connection_id: connection,
                schema: self.state.schema.clone(),
                table: self.state.table.clone(),
            };
            let lease = match target
                .validate()
                .map_err(|_| "Invalid whole-table export target")
                .and_then(|_| model::Lease::new(self.retained.clone(), model::FIELD_BYTES))
            {
                Ok(lease) => lease,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            };
            let view = cx.new(|cx| {
                WholeTableExportView::new(
                    self.host.clone(),
                    target,
                    self.retained.clone(),
                    lease,
                    window,
                    cx,
                )
            });
            self.whole_export.events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        WholeExportEvent::Back => {
                            this.close_whole_export(cx);
                            window.focus(&this.grid.focus_handle(cx), cx);
                        }
                        WholeExportEvent::LoadConfigurations => this.request_whole_config(None, cx),
                        WholeExportEvent::SaveConfiguration(options, revision) => {
                            this.request_whole_config(Some((options.clone(), revision.clone())), cx)
                        }
                        WholeExportEvent::Connect => this.begin_connect(cx),
                        WholeExportEvent::Capture(request) => {
                            this.capture_whole_export(request.clone(), cx)
                        }
                        WholeExportEvent::Cancel => {
                            this.whole_export.config.read.cancel();
                            if this.whole_export.read.busy() {
                                this.whole_export.read.cancel();
                                if let Some(controls) = &this.controls {
                                    controls.cancel();
                                }
                            }
                        }
                        WholeExportEvent::Csv(null_token) => {
                            if let Some(connection) = &this.connection {
                                cx.emit(TableEvent::OpenWholeTableCsv {
                                    connection: connection.clone(),
                                    target: dbunk_lib::backend::csv_transfers::CsvTarget {
                                        schema: this.state.schema.clone(),
                                        table: this.state.table.clone(),
                                    },
                                    null_token: null_token.clone(),
                                });
                            }
                        }
                    }
                    cx.notify();
                },
            ));
            self.whole_export.view = Some(view);
        }
        if let Some(view) = &self.whole_export.view {
            window.focus(&view.read(cx).focus(), cx);
        }
    }
    pub(super) fn close_whole_export(&mut self, cx: &mut Context<Self>) {
        self.whole_export.config.read.cancel();
        if self.whole_export.read.busy() {
            self.whole_export.read.cancel();
            if let Some(controls) = &self.controls {
                controls.cancel();
            }
        }
        self.whole_export.view = None;
        self.whole_export.events = None;
        cx.notify();
    }
    fn capture_whole_export(&mut self, request: TableExportRequest, cx: &mut Context<Self>) {
        if !self.editable
            || self.busy
            || self.changes.read(cx).navigation_blocked()
            || self.whole_export.view.is_none()
        {
            return;
        }
        if request.schema != self.state.schema || request.table != self.state.table {
            return;
        }
        let result = (|| {
            let controls = self
                .controls
                .as_ref()
                .ok_or("Connect before capturing the table")?;
            let id = self.whole_export.read.begin()?;
            if let Err(error) = controls.send(TableCommand::WholeTableExport(id, request)) {
                self.whole_export.read.clear();
                return Err(error);
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.busy = true;
                self.status = "Capturing complete table; grid state is not used".into();
            }
            Err(error) => {
                self.status = format!("Complete table capture refused: {error}");
                if let Some(view) = &self.whole_export.view {
                    view.update(cx, |view, cx| view.fail(error.into(), cx));
                }
            }
        }
        cx.notify();
    }
    pub(super) fn settle_whole_export(
        &mut self,
        id: u64,
        result: Result<TableExportCapture, Arc<DataError>>,
        cx: &mut Context<Self>,
    ) {
        let was_busy = self.whole_export.read.busy();
        let accept = self.whole_export.read.settle(id);
        let settled = was_busy && !self.whole_export.read.busy();
        if settled {
            self.busy = false;
        }
        if !accept {
            // A stale reply must not replace another request's status. A
            // matching cancelled reply has settled its owned read and may do so.
            if settled {
                self.status = "Complete table capture cancelled; no new capture accepted".into();
                if let Some(view) = &self.whole_export.view {
                    view.update(cx, |view, cx| view.fail(self.status.clone(), cx));
                }
                cx.notify();
            }
            return;
        }
        let result = result.map_err(|error| format!("Complete table capture refused: {error:?}"));
        let status = match result {
            Ok(capture) => {
                let rows = capture.data().rows.len();
                let columns = capture.data().columns.len();
                if let Some(view) = &self.whole_export.view {
                    match view.update(cx, |view, cx| view.receive(capture, cx)) {
                        Ok(()) => Ok(format!(
                            "Complete table captured: {rows} rows, {columns} columns"
                        )),
                        Err(error) => Err(format!("Complete table capture refused: {error}")),
                    }
                } else {
                    Ok("Complete table capture discarded; export view closed".into())
                }
            }
            Err(error) => Err(error),
        };
        self.status = match status {
            Ok(message) => message,
            Err(error) => {
                if let Some(view) = &self.whole_export.view {
                    view.update(cx, |view, cx| view.fail(error.clone(), cx));
                }
                error
            }
        };
        cx.notify();
    }
}

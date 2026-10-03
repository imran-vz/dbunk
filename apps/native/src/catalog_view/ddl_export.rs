//! DDL export is a read-only child of the connection-bound Objects document.
//! Dequeued artifacts retain their shared allowance even while a tab is inactive.
use super::*;
use crate::ddl_export_view::{DdlExportEvent, DdlExportRuntime, DdlExportView};
use dbunk_lib::backend::{
    data::DataError,
    ddl_export::{DdlExportArtifact, DdlExportRequest},
};

#[derive(Default)]
pub(super) struct DdlExport {
    pub view: Option<Entity<DdlExportView>>,
    pub incoming: Option<Incoming>,
    pub current: bool,
    pub show: bool,
    events: Option<gpui::Subscription>,
}
pub(super) struct Incoming {
    artifact: Option<DdlExportArtifact>,
    budget: Rc<Cell<usize>>,
    retained: usize,
}
impl Incoming {
    fn new(artifact: DdlExportArtifact, budget: Rc<Cell<usize>>) -> Result<Self, &'static str> {
        let retained = artifact
            .checked_heap_bytes()
            .ok_or("DDL response is invalid or exceeds its bounds; previous capture retained")?;
        if retained > (128 * 1024 * 1024usize).saturating_sub(budget.get()) {
            return Err("DDL export needs shared memory; clear a capture and refresh");
        }
        budget.set(budget.get() + retained);
        Ok(Self {
            artifact: Some(artifact),
            budget,
            retained,
        })
    }
}
impl Drop for Incoming {
    fn drop(&mut self) {
        self.budget
            .set(self.budget.get().saturating_sub(self.retained));
    }
}
impl CatalogView {
    pub(super) fn show_ddl_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.connection.clone() else {
            return;
        };
        if self.ddl_export.view.is_none() {
            let view = cx.new(|cx| {
                DdlExportView::new(
                    self.host.clone(),
                    connection,
                    self.budget.clone(),
                    window,
                    cx,
                )
            });
            self.ddl_export.events = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        DdlExportEvent::Back => {
                            this.ddl_export.show = false;
                            this.previous_focus = None;
                            window.focus(&this.buttons[12], cx);
                        }
                        DdlExportEvent::Connect => this.begin_connect(cx),
                        DdlExportEvent::Refresh(request) => this.load_ddl_export(request.clone()),
                        DdlExportEvent::Cancel => this.activate(Action::Cancel, window, cx),
                    }
                    cx.notify();
                },
            ));
            self.ddl_export.view = Some(view);
        }
        self.ddl_export.show = true;
        if let Some(view) = &self.ddl_export.view {
            window.focus(&view.read(cx).focus(cx), cx);
        }
    }
    fn load_ddl_export(&mut self, request: DdlExportRequest) {
        if !self.ready || !self.enabled(Action::Refresh) {
            return;
        }
        self.ddl_export.current = false;
        self.ddl_export.incoming = None;
        self.next = self.next.wrapping_add(1);
        match self
            .controls
            .as_ref()
            .ok_or("Connect first")
            .and_then(|controls| controls.send(TableCommand::DdlExport(self.next, request)))
        {
            Ok(()) => {
                self.pending = Some(self.next);
                self.busy = true;
                self.cancellation_requested = false;
                self.failure = None;
                self.status = "Reading DDL export metadata; previous capture may be stale".into();
            }
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn settle_ddl_export(&mut self, result: Result<DdlExportArtifact, Arc<DataError>>) {
        self.pending = None;
        self.busy = false;
        match result {
            Ok(artifact) => match Incoming::new(artifact, self.budget.clone()) {
                Ok(incoming) => self.ddl_export.incoming = Some(incoming),
                Err(error) => self.status = error.into(),
            },
            Err(error) => {
                let error = match error.as_ref() {
                    DataError::Catalog(error) => error.to_string(),
                    error => format!("{error:?}"),
                };
                self.status = format!("DDL export refused: {error}; previous capture retained");
            }
        }
    }
    pub(super) fn sync_ddl_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.ddl_export.view else {
            return;
        };
        if let Some(mut incoming) = self.ddl_export.incoming.take() {
            let artifact = incoming.artifact.take().expect("admitted DDL reply");
            match view.update(cx, |view, cx| view.receive(artifact, window, cx)) {
                Ok(()) => {
                    self.ddl_export.current = true;
                    self.status =
                        "DDL artifact captured; inspect its reconstruction omissions before use"
                            .into();
                }
                Err(error) => self.status = error.into(),
            }
        }
        view.update(cx, |view, cx| {
            view.sync(
                DdlExportRuntime {
                    ready: self.ready,
                    busy: self.busy
                        || self.schema_busy
                        || self.maintenance_busy
                        || self.table_ddl_busy,
                    can_cancel: self.busy && self.pending.is_some() && !self.cancellation_requested,
                    editable: self.editable,
                    capture_current: self.ddl_export.current,
                    status: &self.status,
                },
                cx,
            )
        });
    }
}

use super::*;
use dbunk_lib::backend::export_configurations::ExportConfigurationsCapture;
pub(super) struct Configurations {
    pub data: ExportConfigurationsCapture,
    _lease: Rc<Lease>,
}
impl WholeTableExportView {
    pub fn sync_config_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        if self.config_busy != pending {
            self.config_busy = pending;
            cx.notify();
        }
    }
    pub fn receive_configurations(
        &mut self,
        data: ExportConfigurationsCapture,
        saved: bool,
        cx: &mut Context<Self>,
    ) -> Result<(), &'static str> {
        let bytes = data
            .checked_heap_bytes()
            .ok_or("Saved export configurations exceed bounds")?;
        let lease = Lease::new(self.budget.clone(), bytes)?;
        self.configurations = Some(Configurations {
            data,
            _lease: lease,
        });
        self.status=if saved{"Configuration saved for the submitted options. Current options were not changed."}else{"Configurations loaded. Load latest explicitly applies the newest saved recipe for this exact connection and table."}.into();
        cx.notify();
        Ok(())
    }
    pub(super) fn target(&self) -> ExportTarget {
        ExportTarget {
            connection_id: self.connection.clone(),
            schema: self.schema.clone(),
            table: self.table.clone(),
        }
    }
    pub(super) fn save_configuration(&mut self, cx: &mut Context<Self>) {
        match self.raw_options(cx) {
            Ok(options) => {
                if let Some(capture) = &self.configurations {
                    cx.emit(WholeExportEvent::SaveConfiguration(
                        options,
                        capture.data.revision(),
                    ));
                }
            }
            Err(error) => self.status = error.into(),
        }
    }
    pub(super) fn load_latest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.target();
        let Some(record) = self
            .configurations
            .as_ref()
            .and_then(|c| c.data.latest(&target))
        else {
            return;
        };
        let options = record.options.clone();
        if let Err(error) = self.null.update(cx, |field, cx| {
            field.set_value(options.null_token, window, cx)
        }) {
            self.status = error.into();
            return;
        }
        self.format = FORMATS
            .iter()
            .position(|(format, _)| *format == options.format)
            .unwrap();
        self.encoding = options.encoding;
        self.compression = options.compression;
        // Loading a recipe is not an execution or permission token. Clear any
        // historical source so its next run necessarily captures fresh rows.
        self.capture = None;
        self.status="Latest configuration loaded. Capture whole table or open CSV export explicitly; a new file picker is always required.".into();
    }
}

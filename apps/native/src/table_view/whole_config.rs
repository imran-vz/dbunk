//! Profile-local recipes use the existing joined SQLite worker. This lane stays
//! with the table document across child close; cancelled replies are discarded.
use super::*;
use crate::{
    controller::{LibraryCommand, LibraryControls, LibraryDelivery, LibraryReply},
    whole_table_export_model::ReadState,
};
use dbunk_lib::backend::export_configurations::{
    ExportConfigurationsRevision, ExportOptions, ExportTarget,
};
#[derive(Default)]
pub(super) struct ConfigLane {
    controls: Option<LibraryControls>,
    receiver: Option<async_channel::Receiver<LibraryDelivery>>,
    pub read: ReadState,
    saved: bool,
}
impl ConfigLane {
    fn reset_closed(&mut self) {
        if self
            .controls
            .as_ref()
            .is_some_and(LibraryControls::is_closed)
            || self
                .receiver
                .as_ref()
                .is_some_and(async_channel::Receiver::is_closed)
        {
            self.controls = None;
            self.receiver = None;
            self.read.clear();
            self.saved = false;
        }
    }
    pub fn has_pending(&self) -> bool {
        self.receiver
            .as_ref()
            .is_some_and(|r| !r.is_empty() || (r.is_closed() && self.read.busy()))
    }
}
impl Drop for ConfigLane {
    fn drop(&mut self) {
        if let Some(controls) = &self.controls {
            controls.stop();
        }
    }
}
impl TableView {
    pub(super) fn request_whole_config(
        &mut self,
        save: Option<(ExportOptions, ExportConfigurationsRevision)>,
        cx: &mut Context<Self>,
    ) {
        self.whole_export.config.reset_closed();
        if !self.editable
            || self.whole_export.view.is_none()
            || self.whole_export.config.read.busy()
        {
            return;
        }
        let result = (|| {
            if self.whole_export.config.controls.is_none() {
                let (controls, receiver) =
                    self.host.open_library(self.id.clone(), self.wake.clone())?;
                self.whole_export.config.controls = Some(controls);
                self.whole_export.config.receiver = Some(receiver);
            }
            let id = self.whole_export.config.read.begin()?;
            let saved = save.is_some();
            let command = if let Some((options, revision)) = save {
                LibraryCommand::SaveExportConfiguration(
                    id,
                    ExportTarget {
                        connection_id: self.connection.clone().ok_or("Choose a connection")?,
                        schema: self.state.schema.clone(),
                        table: self.state.table.clone(),
                    },
                    options,
                    revision,
                )
            } else {
                LibraryCommand::LoadExportConfigurations(id)
            };
            self.whole_export
                .config
                .controls
                .as_ref()
                .unwrap()
                .send(command)?;
            self.whole_export.config.saved = saved;
            Ok::<_, &'static str>(())
        })();
        if let Err(error) = result {
            self.whole_export.config.read.clear();
            if let Some(view) = &self.whole_export.view {
                view.update(cx, |view, cx| view.fail(error.into(), cx));
            }
        }
        cx.notify();
    }
    pub(super) fn drain_whole_config(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(receiver) = &self.whole_export.config.receiver else {
            return false;
        };
        let delivery = match receiver.try_recv() {
            Ok(delivery) => delivery,
            Err(_) => {
                if receiver.is_closed() && self.whole_export.config.read.busy() {
                    self.whole_export.config.read.clear();
                    if let Some(view) = &self.whole_export.view {
                        view.update(cx,|view,cx|view.fail("Configuration worker closed; reload to reconcile any admitted save".into(),cx));
                    }
                    cx.notify();
                    return true;
                }
                return false;
            }
        };
        let result = match delivery.result {
            Ok(LibraryReply::ExportConfigurations(id, result)) => {
                if !self.whole_export.config.read.settle(id) {
                    cx.notify();
                    return true;
                }
                result
            }
            Err(error) => {
                self.whole_export.config.read.clear();
                Err(error)
            }
            _ => {
                self.whole_export.config.read.clear();
                Err("Unexpected local configuration reply".into())
            }
        };
        if let Some(view) = &self.whole_export.view {
            view.update(cx, |view, cx| match result {
                Ok(data) => {
                    if let Err(error) =
                        view.receive_configurations(data, self.whole_export.config.saved, cx)
                    {
                        view.fail(error.into(), cx);
                    }
                }
                Err(error) => view.fail(error, cx),
            });
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whole_table_export_closed_local_lane_releases_pending_and_fences_late_reply() {
        let (send, receive) = async_channel::bounded(1);
        let mut lane = ConfigLane {
            controls: None,
            receiver: Some(receive),
            read: ReadState::default(),
            saved: false,
        };
        let old = lane.read.begin().unwrap();
        lane.reset_closed();
        assert!(lane.read.busy());
        drop(send);
        lane.reset_closed();
        assert!(lane.receiver.is_none());
        assert!(!lane.read.busy());
        let new = lane.read.begin().unwrap();
        assert!(new > old);
        assert!(!lane.read.settle(old));
        assert!(lane.read.settle(new));
    }
}

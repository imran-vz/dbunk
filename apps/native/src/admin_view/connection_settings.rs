//! Local metadata is supplied by Workspace reload, independently of remote reads.
use super::*;
use crate::connection_settings_view::{ConnectionSettingsView, SettingsEvent};
use dbunk_lib::backend::DevelopmentConnection;
pub(super) struct Settings {
    pub view: Entity<ConnectionSettingsView>,
    pub show: bool,
    events: Option<gpui::Subscription>,
}
impl Settings {
    pub fn new(budget: Rc<Cell<usize>>, cx: &mut Context<AdminView>) -> Self {
        Self {
            view: cx.new(|cx| ConnectionSettingsView::new(budget, cx)),
            show: false,
            events: None,
        }
    }
}
impl AdminView {
    pub fn set_connection_metadata(
        &mut self,
        records: &[DevelopmentConnection],
        cx: &mut Context<Self>,
    ) {
        let id = self.connection.as_deref();
        let record = id.and_then(|id| records.iter().find(|record| record.id == id));
        self.connection_settings
            .view
            .update(cx, |view, cx| view.receive(record, id, cx));
        cx.notify();
    }
    pub(super) fn show_connection_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection_settings.events.is_none() {
            self.connection_settings.events = Some(cx.subscribe_in(
                &self.connection_settings.view,
                window,
                |this, _, event, window, cx| {
                    match event {
                        SettingsEvent::Back => {
                            this.connection_settings.show = false;
                            window.focus(&this.buttons[22], cx);
                        }
                        SettingsEvent::Edit(id)
                            if this.editable && this.connection.as_ref() == Some(id) =>
                        {
                            cx.emit(AdminEvent::EditConnection(id.clone()))
                        }
                        _ => {}
                    }
                    cx.notify();
                },
            ));
        }
        self.connection_settings.show = true;
        window.focus(&self.connection_settings.view.read(cx).focus(), cx);
    }
}

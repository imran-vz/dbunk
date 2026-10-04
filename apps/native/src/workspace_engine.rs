//! Plan 031 step 4: routes connection and tab commands to engine lanes
//! (`engine_lane.rs`) for connections the PostgreSQL document model does not
//! serve. A lane is created on first selection and kept while its connection
//! exists, so switching connections never drops a session.
use super::*;
use crate::engine_lane::EngineLane;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct EngineLanes(BTreeMap<String, (EngineLane, Subscription)>);

impl Workspace {
    /// The lane shown for the selected connection, if it has one.
    pub(super) fn engine_lane(&self) -> Option<&EngineLane> {
        self.engine_lane_of(self.selected_connection.as_deref()?)
    }

    pub(super) fn engine_lane_of(&self, id: &str) -> Option<&EngineLane> {
        self.engine_lanes.0.get(id).map(|(lane, _)| lane)
    }

    fn lane_connection(&self, id: &str) -> Option<&DevelopmentConnection> {
        self.connections
            .iter()
            .find(|connection| connection.id == id && EngineLane::supports(connection))
    }

    /// Handles `operation` when it targets an engine lane; false otherwise.
    pub(super) fn engine_activate(
        &mut self,
        operation: &Operation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match operation {
            Operation::SelectConnection(id) => {
                let Some(connection) = self.lane_connection(id).cloned() else {
                    return false;
                };
                self.selected_connection = Some(id.clone());
                if !self.engine_lanes.0.contains_key(id) {
                    let Some(lane) = EngineLane::new(&connection, self.host.clone(), window, cx)
                    else {
                        return false;
                    };
                    let observer = lane.observe(cx);
                    self.engine_lanes.0.insert(id.clone(), (lane, observer));
                }
                if let Some(lane) = self.engine_lane_of(id).cloned() {
                    lane.connect(window, cx);
                }
                cx.notify();
                true
            }
            Operation::DisconnectConnection(id) => match self.engine_lane_of(id).cloned() {
                Some(lane) => {
                    lane.disconnect(cx);
                    true
                }
                None => false,
            },
            operation => {
                let Some(lane) = self.engine_lane().cloned() else {
                    return false;
                };
                match operation {
                    // A PostgreSQL tab chosen elsewhere (palette) leaves the lane.
                    Operation::SelectDocument(id) if !lane.has_tab(id, cx) => {
                        if let Some(document) = self
                            .documents
                            .iter()
                            .find(|document| &document.metadata.id == id)
                        {
                            self.selected_connection = document.metadata.connection_id.clone();
                        }
                        return false;
                    }
                    Operation::Rename
                    | Operation::Pin
                    | Operation::Move(_)
                    | Operation::Library(_)
                    | Operation::SaveQuery
                    | Operation::Clear => {
                        self.message = Some("Available for PostgreSQL connections only".into());
                    }
                    Operation::New => lane.new_tab(window, cx),
                    Operation::Close => lane.close_active_tab(cx),
                    Operation::CloseDocument(id) if lane.has_tab(id, cx) => lane.close_tab(id, cx),
                    Operation::SelectDocument(id) if lane.has_tab(id, cx) => {
                        lane.select_tab(id, window, cx)
                    }
                    Operation::Next => lane.cycle_tabs(false, window, cx),
                    Operation::Previous => lane.cycle_tabs(true, window, cx),
                    Operation::Connect => lane.connect(window, cx),
                    Operation::Disconnect => lane.disconnect(cx),
                    Operation::OpenTable => {
                        self.message = Some("Open tables from the sidebar tree".into());
                    }
                    _ => return false,
                }
                cx.notify();
                true
            }
        }
    }

    /// Drops lanes whose connection was deleted or became unsupported; a
    /// dropped lane closes its session.
    pub(super) fn sync_engine_lanes(&mut self) {
        let connections = &self.connections;
        self.engine_lanes.0.retain(|id, _| {
            connections
                .iter()
                .any(|connection| &connection.id == id && EngineLane::supports(connection))
        });
    }
}

use super::*;

/// Update geometry and its persisted position together. All fallible validation
/// precedes the record update; Scene itself preserves geometry on refusal.
pub(super) fn move_recorded_node(
    scene: &mut Scene,
    value: &mut SchemaMapPreferences,
    selection: Selection,
    delta: MapPoint,
) -> Result<Selection, &'static str> {
    let Selection::Node { identity, key } = selection else {
        return Err("Select a table to move; foreign-key edges follow their tables");
    };
    if !scene.accepts(selection) || value.database_oid != identity.database_oid {
        return Err("Map movement belongs to an older capture or database");
    }
    if !position_record_available(value, identity) {
        return Err(
            "Saved positions are full. Reset layout explicitly before moving a new table; no geometry was changed",
        );
    }
    let node = scene
        .nodes()
        .iter()
        .find(|node| node.identity == identity)
        .ok_or("Selected table no longer exists in this map")?;
    let position = MapPoint {
        x: node.bounds.x + delta.x,
        y: node.bounds.y + delta.y,
    };
    let next = SceneKey {
        layout_revision: key
            .layout_revision
            .checked_add(1)
            .ok_or("Layout identity exhausted; reopen this map")?,
        ..key
    };
    let slot = value
        .positions
        .iter()
        .position(|saved| saved.identity == identity);
    if slot.is_none() {
        value
            .positions
            .try_reserve_exact(1)
            .map_err(|_| "Map position storage allocation refused")?;
    }
    scene.move_node(next, identity, position)?;
    let saved = SavedPosition { identity, position };
    match slot {
        Some(index) => value.positions[index] = saved,
        None => value.positions.push(saved),
    }
    Ok(Selection::Node {
        identity,
        key: next,
    })
}

pub(super) fn position_record_available(
    value: &SchemaMapPreferences,
    identity: SchemaMapIdentity,
) -> bool {
    value.positions.len() < MAX_MAP_POSITIONS
        || value.positions.iter().any(|p| p.identity == identity)
}

impl SchemaMapView {
    pub(super) fn movement_allowed(&self) -> bool {
        self.editable
            && self._ui_lease.is_some()
            && self.ready
            && self.current
            && !self.opening
            && self.pending.is_none()
            && self.revision.is_some()
    }
    pub(super) fn nudge_selected(&mut self, delta: MapPoint, cx: &mut Context<Self>) {
        if !self.movement_allowed() || self.drag.is_some() {
            self.status = "Movement unavailable: finish the current action, then Refresh if the map or saved settings are stale".into();
            cx.notify();
            return;
        }
        let (Some(scene), Some(value), Some(selection)) =
            (&mut self.scene, &mut self.working, self.selection)
        else {
            self.status = "Select a table in the map or object list before moving it".into();
            cx.notify();
            return;
        };
        match move_recorded_node(scene, value, selection, delta) {
            Ok(selection) => {
                self.selection = Some(selection);
                self.dirty = true;
                self.update_details();
                self.save_preferences(cx);
            }
            Err(error) => self.report(error),
        }
        cx.notify();
    }
}

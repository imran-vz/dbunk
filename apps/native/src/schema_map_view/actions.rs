use super::*;
impl SchemaMapView {
    pub(super) fn enabled(&self, action: Action) -> bool {
        if !self.editable || self._ui_lease.is_none() {
            return false;
        }
        let idle = self.pending.is_none() && !self.opening;
        match action {
            Action::Connect => idle && self.controls.is_none() && self.connection.is_some(),
            Action::Refresh => idle && self.ready && !self.dirty,
            Action::Cancel => matches!(
                self.pending,
                Some(Pending::Graph {
                    cancelled: false,
                    ..
                })
            ),
            Action::Clear => idle,
            Action::Fit | Action::ZoomIn | Action::ZoomOut => self.scene.is_some(),
            Action::Routing
            | Action::Attributes
            | Action::Types
            | Action::Nulls
            | Action::Comments
            | Action::ResetLayout => idle && self.scene.is_some() && self.ready && self.current,
            Action::Save => {
                idle && self.ready && self.current && self.dirty && self.revision.is_some()
            }
            Action::Defaults => idle && self.ready && self.current && self.revision.is_some(),
            Action::Open => {
                idle && self.ready
                    && self.current
                    && matches!(self.selection, Some(Selection::Node { .. }))
            }
            Action::Copy => !self.detail_text.is_empty(),
            Action::Glossary => true,
            Action::Previous => self.detail_page > 0 && self.selection.is_some(),
            Action::Next => self.detail_next,
            Action::Svg | Action::Png => {
                self.scene.is_some() && !self.file_busy && self.drag.is_none()
            }
            Action::CancelFile => self.file_busy,
            Action::Scope(_) => idle && !self.dirty,
        }
    }
    pub(super) fn activate(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.composing(window, cx) || !self.enabled(action) {
            return;
        }
        match action {
            Action::Connect => self.begin_connect(cx),
            Action::Refresh => self.refresh(cx),
            Action::Clear => self.clear_results(cx),
            Action::Cancel => {
                if let Some(Pending::Graph { cancelled, .. }) = &mut self.pending {
                    *cancelled = true;
                    if let Some(controls) = &self.controls {
                        controls.cancel();
                    }
                    self.status = "Cancellation requested; waiting for owned read cleanup".into();
                }
            }
            Action::Fit => self.fit(),
            Action::ZoomIn | Action::ZoomOut => {
                let point = MapPoint {
                    x: self.viewport.width as f64 / 2.,
                    y: self.viewport.height as f64 / 2.,
                };
                if let Ok(camera) = self.camera.zoom_at(
                    point,
                    self.camera.zoom
                        * if action == Action::ZoomIn {
                            1.2
                        } else {
                            1. / 1.2
                        },
                ) {
                    self.camera = camera;
                }
            }
            Action::Routing
            | Action::Attributes
            | Action::Types
            | Action::Nulls
            | Action::Comments
            | Action::ResetLayout => {
                if let Some(mut value) = self.working.clone() {
                    match action {
                        Action::Routing => {
                            value.prefs.routing = match value.prefs.routing {
                                MapRouting::Curve => MapRouting::Step,
                                MapRouting::Step => MapRouting::Curve,
                            }
                        }
                        Action::Attributes => {
                            value.prefs.attributes = match value.prefs.attributes {
                                MapAttributes::All => MapAttributes::KeysOnly,
                                MapAttributes::KeysOnly => MapAttributes::None,
                                MapAttributes::None => MapAttributes::All,
                            }
                        }
                        Action::Types => value.prefs.show_types = !value.prefs.show_types,
                        Action::Nulls => value.prefs.show_nulls = !value.prefs.show_nulls,
                        Action::Comments => value.prefs.show_comments = !value.prefs.show_comments,
                        Action::ResetLayout => value.positions.clear(),
                        _ => {}
                    }
                    if self.replace_preferences(value, cx) {
                        if action == Action::ResetLayout {
                            self.fit();
                        }
                        self.save_preferences(cx);
                    }
                }
            }
            Action::Save => self.save_preferences(cx),
            Action::Defaults => self.reset_preferences(cx),
            Action::Open => self.open_selected(cx),
            Action::Glossary => {
                self.selection = None;
                self.glossary = true;
                self.detail_page = 0;
                self.detail_next = false;
                self.detail_text="Relationship glossary\n\nSelect a table, then Shift+Arrow moves it 10 world units from canvas or list focus. Each nudge saves; wait for acknowledgement before another move. Plain arrows pan the canvas or select list rows.\n\nA bar marks one; a circle marks optional (zero or one); a crow's foot marks many.\nOne-to-one: referencing columns contain a valid, unconditional unique index key. Otherwise the relationship is one-to-many. Nullability indicates whether a referencing key can be absent.\nJunction marks an identity covered by at least two distinct outgoing foreign keys, with no single key covering that identity alone.\nComposite column pairs remain in declared order. A label may be shortened on canvas; selected details expose every exact pair and metadata value across pages.\nThese are catalog constraints, not measured row counts. External nodes are fully described targets outside the selected schema. Table scope includes direct neighbors and only focal edges. Inherited FK/trigger copies are omitted.\nSVG/PNG export the current viewport on white, including offscreen clipping. PNG uses bounded bundled fonts and may refuse missing glyphs; SVG permits system-font fallback.".into();
                window.focus(&self.details, cx);
            }
            Action::Copy => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(self.detail_text.clone()));
                let copied=cx.read_from_clipboard().is_some_and(|item|matches!(item.entries(),[gpui::ClipboardEntry::String(text)] if text.text()==&self.detail_text));
                self.status = if copied {
                    "Copied exact displayed detail page"
                } else {
                    "Clipboard write could not be verified"
                }
                .into();
            }
            Action::Previous | Action::Next => {
                self.detail_page = if action == Action::Next {
                    self.detail_page + 1
                } else {
                    self.detail_page.saturating_sub(1)
                };
                self.update_details();
            }
            Action::Svg | Action::Png => self.save_image(action == Action::Png, window, cx),
            Action::CancelFile => {
                if let Some(token) = &self.file_cancel {
                    token.cancel();
                }
                self.status = "File cancellation requested; waiting for joined completion".into();
            }
            Action::Scope(scope) => {
                self.scope = scope;
                self.status="Setup scope changed. Refresh explicitly reads it; retained map scope remains shown below".into();
            }
        }
        cx.notify();
    }
    pub(super) fn replace_preferences(
        &mut self,
        value: SchemaMapPreferences,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(scene) = &self.scene else {
            return false;
        };
        let key = scene.key();
        let Some(revision) = key.layout_revision.checked_add(1) else {
            self.status = "Layout identity exhausted; reopen this map".into();
            return false;
        };
        match scene.rebuild(
            SceneKey {
                layout_revision: revision,
                ..key
            },
            value.prefs,
            &value.positions,
        ) {
            Ok(scene) => {
                self.selection = self.selection.and_then(|s| match s {
                    Selection::Node { identity, .. } => scene.node_selection(identity),
                    Selection::Edge { identity, .. } => scene.edge_selection(identity),
                });
                self.scene = Some(scene);
                self.working = Some(value);
                self.dirty = true;
                self.status = "Map settings changed locally; not yet saved".into();
                self.update_details();
                cx.notify();
                true
            }
            Err(error) => {
                self.report(error);
                false
            }
        }
    }
    pub(super) fn select(&mut self, selection: Option<Selection>) {
        self.glossary = false;
        self.selection = selection
            .filter(|selection| self.scene.as_ref().is_some_and(|s| s.accepts(*selection)));
        self.detail_page = 0;
        self.update_details();
    }
    pub(super) fn update_details(&mut self) {
        self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
        let text = match (&self.scene, self.selection) {
            (Some(scene), Some(selection)) => scene.details_page(selection, self.detail_page),
            _ => {
                self.detail_text.clear();
                self.detail_next = false;
                return;
            }
        };
        match text {
            Ok(page) => {
                self.detail_page = page.page;
                self.detail_text = page.text;
                self.detail_next = page.next;
            }
            Err(error) => {
                self.detail_text = error.into();
                self.detail_next = false;
            }
        }
    }
    pub(super) fn count(&self) -> usize {
        self.scene
            .as_ref()
            .map_or(0, |s| s.nodes().len() + s.edges().len())
    }
    pub(super) fn selection_at(&self, index: usize) -> Option<Selection> {
        let scene = self.scene.as_ref()?;
        if index < scene.nodes().len() {
            scene.node_selection(scene.nodes()[index].identity)
        } else {
            scene
                .edges()
                .get(index - scene.nodes().len())
                .and_then(|e| scene.edge_selection(e.identity))
        }
    }
    pub(super) fn selected_index(&self) -> Option<usize> {
        let scene = self.scene.as_ref()?;
        match self.selection? {
            Selection::Node { identity, .. } => {
                scene.nodes().iter().position(|n| n.identity == identity)
            }
            Selection::Edge { identity, .. } => scene
                .edges()
                .iter()
                .position(|e| e.identity == identity)
                .map(|i| i + scene.nodes().len()),
        }
    }
    pub(super) fn row_label(&self, index: usize) -> String {
        let Some(scene) = &self.scene else {
            return String::new();
        };
        if let Some(node) = scene.nodes().get(index) {
            format!(
                "Table {} · OID {}{}{} · {} triggers",
                node.title,
                node.identity.relation_oid,
                if node.external {
                    " · external target"
                } else {
                    ""
                },
                if node.junction { " · junction" } else { "" },
                node.trigger_count
            )
        } else {
            scene
                .edges()
                .get(index.saturating_sub(scene.nodes().len()))
                .map(|edge| {
                    format!(
                        "Foreign key OID {} · {}",
                        edge.identity.constraint_oid, edge.label
                    )
                })
                .unwrap_or_default()
        }
    }
    pub(super) fn focus_order(&self, cx: &gpui::App) -> Vec<FocusHandle> {
        let mut order = self
            .buttons
            .iter()
            .enumerate()
            .filter(|(i, _)| self.enabled(ACTIONS[*i].0))
            .map(|(_, f)| f.clone())
            .collect::<Vec<_>>();
        order.extend(
            self.scope_buttons
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    self.enabled(Action::Scope(
                        [Scope::Database, Scope::Schema, Scope::Relation][*i],
                    ))
                })
                .map(|(_, focus)| focus.clone()),
        );
        if self.scope != Scope::Database {
            order.extend(self.fields.first().map(|f| f.focus_handle(cx)));
        }
        if self.scope == Scope::Relation {
            order.extend(self.fields.get(1).map(|f| f.focus_handle(cx)));
        }
        if self.scene.is_some() {
            order.push(self.canvas_focus.clone());
            order.push(self.list.clone());
        }
        if self.selection.is_some() || self.glossary {
            order.push(self.details.clone());
        }
        if order.is_empty() {
            order.push(self.root.clone());
        }
        order
    }
    pub(super) fn focus_control(
        &mut self,
        backward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let order = self.focus_order(cx);
        let current = order
            .iter()
            .position(|focus| focus.contains_focused(window, cx));
        let next = if backward {
            current.map_or(order.len() - 1, |i| (i + order.len() - 1) % order.len())
        } else {
            current.map_or(0, |i| (i + 1) % order.len())
        };
        window.focus(&order[next], cx);
    }
    pub(super) fn key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let m = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if m.platform
            && !m.alt
            && !m.control
            && key == "c"
            && (self.details.is_focused(window) || self.list.is_focused(window))
        {
            self.activate(Action::Copy, window, cx);
            cx.stop_propagation();
            return;
        }
        if m.platform || m.alt || m.control {
            return;
        }
        if m.shift && (self.canvas_focus.is_focused(window) || self.list.is_focused(window)) {
            let delta = match key {
                "up" => Some(MapPoint { x: 0., y: -10. }),
                "down" => Some(MapPoint { x: 0., y: 10. }),
                "left" => Some(MapPoint { x: -10., y: 0. }),
                "right" => Some(MapPoint { x: 10., y: 0. }),
                _ => None,
            };
            if let Some(delta) = delta {
                self.nudge_selected(delta, cx);
                cx.stop_propagation();
                return;
            }
        }
        if key == "tab" {
            self.focus_control(m.shift, window, cx);
            cx.stop_propagation();
            return;
        }
        if self.details.is_focused(window) {
            match key {
                "escape" => window.focus(&self.list, cx),
                "pageup" => self.activate(Action::Previous, window, cx),
                "pagedown" => self.activate(Action::Next, window, cx),
                "up" | "down" | "left" | "right" | "home" | "end" => {
                    let mut offset = self.detail_scroll.offset();
                    let max = self.detail_scroll.max_offset();
                    match key {
                        "up" => offset.y += px(28.),
                        "down" => offset.y -= px(28.),
                        "left" => offset.x += px(40.),
                        "right" => offset.x -= px(40.),
                        "home" => offset = gpui::point(px(0.), px(0.)),
                        "end" => offset.y = -max.y,
                        _ => {}
                    }
                    offset.x = offset.x.max(-max.x).min(px(0.));
                    offset.y = offset.y.max(-max.y).min(px(0.));
                    self.detail_scroll.set_offset(offset);
                    cx.notify();
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if self.canvas_focus.is_focused(window) {
            let delta = match key {
                "up" => Some(MapPoint { x: 0., y: 40. }),
                "down" => Some(MapPoint { x: 0., y: -40. }),
                "left" => Some(MapPoint { x: 40., y: 0. }),
                "right" => Some(MapPoint { x: -40., y: 0. }),
                _ => None,
            };
            if let Some(delta) = delta {
                if let Ok(camera) = self.camera.pan_by(delta) {
                    self.camera = camera;
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            match key {
                "+" | "=" => self.activate(Action::ZoomIn, window, cx),
                "-" => self.activate(Action::ZoomOut, window, cx),
                "f" => self.activate(Action::Fit, window, cx),
                "escape" => {
                    self.select(None);
                    cx.notify();
                }
                _ => return,
            }
            cx.stop_propagation();
            return;
        }
        if !self.list.is_focused(window) {
            return;
        }
        let count = self.count();
        if count == 0 {
            return;
        }
        let index = self.selected_index().unwrap_or(0);
        let next = match key {
            "up" => index.saturating_sub(1),
            "down" => self.selected_index().map_or(0, |i| (i + 1).min(count - 1)),
            "home" => 0,
            "end" => count - 1,
            "pageup" => index.saturating_sub(20),
            "pagedown" => (index + 20).min(count - 1),
            "enter" => {
                if self.selection.is_some() {
                    window.focus(&self.details, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.select(self.selection_at(next));
        self.scroll.scroll_to_item(next, gpui::ScrollStrategy::Top);
        cx.notify();
        cx.stop_propagation();
    }
}

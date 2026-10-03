use super::*;
impl SchemaMapView {
    pub fn begin_connect(&mut self, cx: &mut Context<Self>) {
        if !self.editable || self.controls.is_some() || self.opening {
            return;
        }
        let Some(connection) = &self.connection else {
            self.status = "Select a connection first".into();
            cx.notify();
            return;
        };
        match self
            .host
            .open_table_document(self.id.clone(), connection.clone(), self.wake.clone())
        {
            Ok((controls, receiver)) => {
                self.controls = Some(controls);
                self.receiver = Some(receiver);
                self.opening = true;
                self.ready = false;
                self.status = "Connecting map reader".into();
            }
            Err(error) => self.report(error),
        }
        self.sync_fields(cx);
        cx.notify();
    }
    pub(super) fn request(&self, cx: &gpui::App) -> Result<SchemaMapRequest, &'static str> {
        let field = |index: usize| {
            self.fields
                .get(index)
                .ok_or("Map fields need memory allowance")?
                .read(cx)
                .value(cx)
        };
        let scope = match self.scope {
            Scope::Database => SchemaMapScope::Database,
            Scope::Schema => SchemaMapScope::Schema {
                name: field(0)?,
                expected_oid: None,
            },
            Scope::Relation => SchemaMapScope::Relation {
                schema: field(0)?,
                table: field(1)?,
                expected: None,
            },
        };
        let mut request = SchemaMapRequest {
            scope,
            expected_database_oid: None,
        };
        // Disconnect does not authorize silently adopting replacement OIDs.
        request = preserve_identity(request, self.scene.as_ref().map(Scene::snapshot));
        request
            .validate()
            .map_err(|_| "Enter exact nonempty schema/table names, each at most 63 UTF-8 bytes")?;
        Ok(request)
    }
    pub(super) fn refresh(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(Action::Refresh) {
            return;
        }
        match self.request(cx) {
            Ok(request) => self.send_graph(request, Purpose::Refresh, cx),
            Err(error) => {
                self.report(error);
                cx.notify();
            }
        }
    }
    fn send_graph(&mut self, request: SchemaMapRequest, purpose: Purpose, cx: &mut Context<Self>) {
        let result = self.next_id().and_then(|id| {
            self.controls
                .as_ref()
                .ok_or("Connect first")?
                .send(TableCommand::SchemaMap(id, request.clone()))?;
            if matches!(purpose, Purpose::Refresh) {
                self.current = false;
            }
            self.pending = Some(Pending::Graph {
                id,
                request,
                purpose,
                cancelled: false,
            });
            Ok(())
        });
        match result {
            Ok(()) => {
                self.status = "Reading one complete map snapshot; previous map retained".into()
            }
            Err(error) => self.report(error),
        }
        self.sync_fields(cx);
        cx.notify();
    }
    pub(super) fn open_selected(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(Action::Open) {
            return;
        }
        let (Some(scene), Some(Selection::Node { identity, key })) = (&self.scene, self.selection)
        else {
            return;
        };
        let Some(table) = scene
            .snapshot()
            .tables
            .iter()
            .find(|table| table.identity == identity)
        else {
            return;
        };
        let request = SchemaMapRequest {
            scope: SchemaMapScope::Relation {
                schema: table.schema.clone(),
                table: table.name.clone(),
                expected: Some(identity),
            },
            expected_database_oid: Some(identity.database_oid),
        };
        self.send_graph(
            request,
            Purpose::Open {
                identity,
                scene: key,
            },
            cx,
        );
    }
    pub fn drain_one(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(envelope) = self.receiver.as_ref().and_then(TableReceiver::try_recv) else {
            return false;
        };
        match envelope.into_message() {
            TableMessage::Opened => {
                self.opening = false;
                self.ready = true;
                self.status = "Connected. Refresh explicitly reads map metadata".into();
            }
            TableMessage::SchemaMap(id, result) => {
                if self.pending.as_ref().is_none_or(|p| p.id() != id) {
                    return true;
                }
                let Some(Pending::Graph {
                    request,
                    purpose,
                    cancelled,
                    ..
                }) = self.pending.take()
                else {
                    self.status = "Unexpected map reply; request authority lost".into();
                    self.sync_fields(cx);
                    cx.notify();
                    return true;
                };
                if cancelled {
                    self.status = "Map read cancelled; previous map retained".into();
                } else {
                    match result {
                        Err(error) => {
                            // A failed refresh or target recheck cannot establish current identity.
                            self.current = false;
                            self.report(format!(
                                "Map read failed: {error:?}; previous map retained"
                            ));
                        }
                        Ok(snapshot) => {
                            if snapshot.scope != request.scope
                                || snapshot.checked_heap_bytes().is_none()
                            {
                                self.current = false;
                                self.status =
                                    "Map reply identity or bounds invalid; previous map retained"
                                        .into();
                            } else {
                                match purpose {
                                    Purpose::Open { identity, scene } => {
                                        if self.scene.as_ref().is_some_and(|s| s.key() == scene)
                                            && snapshot.focus == Some(identity)
                                            && self.current
                                        {
                                            if let Some(table) = snapshot
                                                .tables
                                                .iter()
                                                .find(|table| table.identity == identity)
                                                && let Some(connection) = &self.connection
                                            {
                                                cx.emit(SchemaMapEvent::OpenTable {
                                                    connection: connection.clone(),
                                                    schema: table.schema.clone(),
                                                    table: table.name.clone(),
                                                });
                                                self.status="Selected table identity rechecked before opening".into();
                                            }
                                        } else {
                                            self.current = false;
                                            self.status =
                                                "Selected table changed; opening refused".into();
                                        }
                                    }
                                    Purpose::Refresh => {
                                        let bytes = snapshot.checked_heap_bytes().unwrap();
                                        match Lease::new(self.budget.clone(), bytes) {
                                            Err(error) => self.report(error),
                                            Ok(lease) => {
                                                let scope = preference_scope(&snapshot.scope);
                                                self.incoming = Some(Incoming {
                                                    snapshot: Arc::new(snapshot),
                                                    _lease: lease,
                                                });
                                                self.load_preferences(scope, cx);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            TableMessage::MapPreferencesLoad(id, result) => {
                if self.pending.as_ref().is_none_or(|p| p.id() != id) {
                    return true;
                }
                let Some(Pending::Load { scope, .. }) = self.pending.take() else {
                    return true;
                };
                match result {
                    Ok(capture) if self.capture_matches(&capture, &scope) => {
                        self.install_incoming(Some(capture), None, cx)
                    }
                    Ok(_) => self.install_incoming(
                        None,
                        Some("Map preferences returned a different scope".into()),
                        cx,
                    ),
                    Err(error) => self.install_incoming(None, Some(error.to_string()), cx),
                }
            }
            TableMessage::MapPreferencesSave(id, result) => {
                if self.pending.as_ref().is_none_or(|p| p.id() != id) {
                    return true;
                }
                let Some(Pending::Save { scope, value, .. }) = self.pending.take() else {
                    return true;
                };
                match result {
                    Ok(capture)
                        if self.capture_matches(&capture, &scope)
                            && capture.value.as_ref() == Some(&value) =>
                    {
                        self.revision = Some(capture.revision);
                        self.dirty = false;
                        self.status = "Map settings and positions saved to this exact scope".into();
                    }
                    Ok(_) => {
                        self.revision = None;
                        self.status="Save acknowledgement did not match submitted settings; inspect local changes, then explicitly Clear and Refresh".into();
                    }
                    Err(error) => {
                        self.revision = None;
                        self.report(format!(
                            "Map settings remain unsaved: {error}. Inspect local changes, then explicitly Clear and Refresh before another save"
                        ));
                    }
                }
            }
            TableMessage::MapPreferencesReset(id, result) => {
                if self.pending.as_ref().is_none_or(|p| p.id() != id) {
                    return true;
                }
                let Some(Pending::Reset {
                    scope,
                    scene,
                    value,
                    ..
                }) = self.pending.take()
                else {
                    return true;
                };
                match result {
                    Ok(capture)
                        if self.capture_matches(&capture, &scope) && capture.value.is_none() =>
                    {
                        self.revision = Some(capture.revision);
                        self.selection = self.selection.and_then(|selection| match selection {
                            Selection::Node { identity, .. } => scene.node_selection(identity),
                            Selection::Edge { identity, .. } => scene.edge_selection(identity),
                        });
                        self.scene = Some(*scene);
                        self.working = Some(value);
                        self.dirty = false;
                        self.update_details();
                        self.fit();
                        self.status = "This scope reset to native map defaults".into();
                    }
                    Ok(_) => {
                        self.revision = None;
                        self.status =
                            "Reset acknowledgement mismatch; explicitly Clear and Refresh map preferences".into();
                    }
                    Err(error) => {
                        self.revision = None;
                        self.dirty = true;
                        self.report(format!(
                            "Map reset failed: {error}; retained settings may be unsaved"
                        ));
                    }
                }
            }
            TableMessage::Error(error) => self.report(format!("Map worker: {error}")),
            TableMessage::Closed(result) => {
                self.mark_disconnected(cx);
                if let Err(error) = result {
                    self.report(format!("Map reader cleanup failed: {error}"));
                }
            }
            _ => self.status = "Unexpected reply in map reader".into(),
        }
        self.sync_fields(cx);
        cx.notify();
        true
    }
    fn capture_matches(
        &self,
        capture: &SchemaMapPreferencesCapture,
        scope: &SchemaMapPreferenceScope,
    ) -> bool {
        capture.checked_heap_bytes().is_some()
            && capture.scope == *scope
            && Some(capture.connection_id.as_str()) == self.connection.as_deref()
    }
    fn load_preferences(&mut self, scope: SchemaMapPreferenceScope, cx: &mut Context<Self>) {
        let result = self.next_id().and_then(|id| {
            self.controls
                .as_ref()
                .ok_or("Map reader disconnected")?
                .send(TableCommand::MapPreferencesLoad(id, scope.clone()))?;
            self.pending = Some(Pending::Load { id, scope });
            Ok(())
        });
        if let Err(error) = result {
            self.install_incoming(None, Some(error.into()), cx);
        } else {
            self.status = "Map captured; loading this scope's local settings".into();
        }
    }
    fn install_incoming(
        &mut self,
        capture: Option<SchemaMapPreferencesCapture>,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(incoming) = self.incoming.take() else {
            return;
        };
        let key = SceneKey {
            document_generation: self.generation,
            capture_generation: self.sequence,
            layout_revision: 0,
        };
        let mut ignored = false;
        let value = capture
            .as_ref()
            .and_then(|c| c.value.as_ref())
            .filter(|p| {
                let same = p.database_oid == incoming.snapshot.database_oid;
                ignored = !same;
                same
            })
            .cloned()
            .unwrap_or_else(|| SchemaMapPreferences::for_database(incoming.snapshot.database_oid));
        match Scene::new(
            incoming.snapshot.clone(),
            key,
            value.prefs,
            &value.positions,
            self.budget.clone(),
        ) {
            Err(error) => self.report(error),
            Ok(scene) => {
                self.preference_scope = Some(preference_scope(&scene.snapshot().scope));
                self.revision = capture.map(|c| c.revision);
                self.working = Some(value);
                self.scene = Some(scene);
                self.current = true;
                self.dirty = false;
                self.selection = None;
                self.detail_page = 0;
                self.detail_text.clear();
                self.detail_next = false;
                self.glossary = false;
                self.fit_pending = true;
                self.status = if let Some(error) = error {
                    format!(
                        "Map captured; preferences unavailable: {error}. Changes cannot be saved until Refresh succeeds"
                    )
                } else if ignored {
                    "Map captured. Saved positions belong to another database OID; defaults shown without overwriting storage".into()
                } else {
                    "Complete bounded map captured. Constraints describe relationships, not measured row counts".into()
                };
            }
        }
        cx.notify();
    }
    pub(super) fn save_preferences(&mut self, cx: &mut Context<Self>) {
        if !self.enabled(Action::Save) {
            return;
        }
        let (Some(scope), Some(revision), Some(value)) = (
            self.preference_scope.clone(),
            self.revision.clone(),
            self.working.clone(),
        ) else {
            return;
        };
        let result = self.next_id().and_then(|id| {
            self.controls.as_ref().ok_or("Connect first")?.send(
                TableCommand::MapPreferencesSave(id, scope.clone(), revision, value.clone()),
            )?;
            self.pending = Some(Pending::Save { id, scope, value });
            Ok(())
        });
        match result {
            Ok(()) => {
                self.status = "Map settings unsaved until exact storage acknowledgement".into()
            }
            Err(error) => self.report(error),
        }
        self.sync_fields(cx);
        cx.notify();
    }
    pub(super) fn reset_preferences(&mut self, cx: &mut Context<Self>) {
        let (Some(scope), Some(revision), Some(scene)) = (
            self.preference_scope.clone(),
            self.revision.clone(),
            self.scene.as_ref(),
        ) else {
            return;
        };
        let key = scene.key();
        let Some(next) = key.layout_revision.checked_add(1) else {
            self.status = "Layout identity exhausted; reopen tab".into();
            return;
        };
        let value = SchemaMapPreferences::for_database(scene.snapshot().database_oid);
        let scene = match scene.rebuild(
            SceneKey {
                layout_revision: next,
                ..key
            },
            value.prefs,
            &[],
        ) {
            Ok(scene) => Box::new(scene),
            Err(error) => {
                self.report(error);
                return;
            }
        };
        let result = self.next_id().and_then(|id| {
            self.controls.as_ref().ok_or("Connect first")?.send(
                TableCommand::MapPreferencesReset(id, scope.clone(), revision),
            )?;
            self.pending = Some(Pending::Reset {
                id,
                scope,
                scene,
                value,
            });
            Ok(())
        });
        match result {
            Ok(()) => self.status = "Resetting only this exact scope's saved settings".into(),
            Err(error) => self.report(error),
        }
        self.sync_fields(cx);
        cx.notify();
    }
}

pub(super) fn preserve_identity(
    request: SchemaMapRequest,
    captured: Option<&SchemaMapSnapshot>,
) -> SchemaMapRequest {
    match captured {
        Some(snapshot) if preference_scope(&request.scope) == preference_scope(&snapshot.scope) => {
            snapshot.refresh_request()
        }
        _ => request,
    }
}

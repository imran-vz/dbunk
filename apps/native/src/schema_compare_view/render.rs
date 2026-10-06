use super::*;
fn navigation(key: &str, current: Option<usize>, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "up" => Some(current.unwrap_or(0).saturating_sub(1)),
        "down" => Some(current.map_or(0, |index| (index + 1).min(count - 1))),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}
impl SchemaCompareView {
    fn button(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let (action, label) = ACTIONS[index];
        let copy_key = if let Action::Copy(side) = action {
            self.copy_key(side, cx)
        } else {
            None
        };
        let click_key = copy_key.clone();
        let accessibility_key = copy_key.clone();
        let label = if let Some(key) = &copy_key && self.reader.read(cx).state().and_then(|state| state.value(key.value.side)).is_some_and(|reply| matches!(reply,CompareReply::Value { offset,next_offset,.. } if *offset != 0 || *next_offset < key.value.raw_bytes)) { format!("{label} (partial value)") } else { label.to_owned() };
        let enabled = self.enabled(action, cx);
        let weak = cx.weak_entity();
        crate::ui::tool_button(("comparison-action", index), label, None, enabled, false)
            .track_focus(&self.buttons[index])
            .tab_index(0)
            .tab_stop(enabled)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.activate_captured(action, click_key.as_ref(), window, cx)
            }))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| {
                    this.activate_captured(action, accessibility_key.as_ref(), window, cx)
                })
                .ok();
            })
            .into_any_element()
    }
    fn connection_element(&self, side: Side, cx: &Context<Self>) -> AnyElement {
        let index = side_index(side);
        let id = if index == 0 {
            self.source.as_deref()
        } else {
            self.target.as_deref()
        };
        let generation = self.generation;
        let current = self
            .connections
            .as_ref()
            .and_then(|connections| id.and_then(|id| connections.get(id)));
        let label = current.map_or_else(
            || {
                id.map_or_else(
                    || {
                        format!(
                            "{}: choose a stored PostgreSQL connection",
                            side_label(side)
                        )
                    },
                    |id| format!("{}: missing connection {id}", side_label(side)),
                )
            },
            |connection| {
                format!(
                    "{}: {} · {} · {} · {}",
                    side_label(side),
                    connection.name,
                    connection.database,
                    connection.environment,
                    connection.id
                )
            },
        );
        let setup_enabled = self.setup_enabled(cx);
        let count = self
            .connections
            .as_ref()
            .map_or(0, |connections| connections.rows().len());
        div()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .id(("comparison-endpoint", index))
                    .role(Role::Label)
                    .aria_label(label.clone())
                    .child(label),
            )
            .when(self.connections.is_some(), |body| {
                body.child(
                    div()
                        .id(("comparison-connections", index))
                        .role(Role::ListBox)
                        .a11y_synthetic_children(move |builder| {
                            if !setup_enabled {
                                builder.parent_node().set_disabled();
                            }
                        })
                        .aria_label(format!("{} stored PostgreSQL connection", side_label(side)))
                        .track_focus(&self.connection_focus[index])
                        .tab_index(0)
                        .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                            if !this.connection_focus[index].is_focused(window)
                                || generation != this.generation
                            {
                                return;
                            }
                            let Some(connections) = &this.connections else {
                                return;
                            };
                            let current = if index == 0 {
                                &this.source
                            } else {
                                &this.target
                            };
                            let position = connections
                                .rows()
                                .iter()
                                .position(|row| Some(row.id.as_str()) == current.as_deref());
                            if let Some(next) =
                                navigation(&event.keystroke.key, position, connections.rows().len())
                            {
                                let id = connections.rows()[next].id.clone();
                                this.select_connection(side, id, generation, window, cx);
                                this.connection_scroll[index]
                                    .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                                cx.stop_propagation();
                            }
                        }))
                        .child(
                            gpui::uniform_list(
                                ("comparison-connection-items", index),
                                count,
                                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|position| {
                                            let Some(connection) =
                                                this.connections.as_ref().and_then(|connections| {
                                                    connections.rows().get(position)
                                                })
                                            else {
                                                return div().into_any_element();
                                            };
                                            let id = connection.id.clone();
                                            let label = format!(
                                                "{} · {} · {}",
                                                connection.name,
                                                connection.database,
                                                connection.environment
                                            );
                                            let selected = if index == 0 {
                                                this.source.as_deref()
                                            } else {
                                                this.target.as_deref()
                                            } == Some(id.as_str());
                                            let weak = cx.weak_entity();
                                            let accessible_id = id.clone();
                                            div()
                                                .id(("comparison-connection-row", position))
                                                .role(Role::ListBoxOption)
                                                .a11y_synthetic_children(move |builder| {
                                                    if !setup_enabled {
                                                        builder.parent_node().set_disabled();
                                                    }
                                                })
                                                .aria_label(format!("{label} · {id}"))
                                                .aria_selected(selected)
                                                .h(px(26.))
                                                .px_2()
                                                .when(selected, |row| {
                                                    row.bg(crate::style::select())
                                                })
                                                .child(label)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.select_connection(
                                                            side,
                                                            id.clone(),
                                                            generation,
                                                            window,
                                                            cx,
                                                        );
                                                        window.focus(
                                                            &this.connection_focus[index],
                                                            cx,
                                                        );
                                                    },
                                                ))
                                                .on_a11y_action(
                                                    gpui::accesskit::Action::Click,
                                                    move |_, window, cx| {
                                                        weak.update(cx, |this, cx| {
                                                            this.select_connection(
                                                                side,
                                                                accessible_id.clone(),
                                                                generation,
                                                                window,
                                                                cx,
                                                            );
                                                            window.focus(
                                                                &this.connection_focus[index],
                                                                cx,
                                                            );
                                                        })
                                                        .ok();
                                                    },
                                                )
                                                .into_any_element()
                                        })
                                        .collect()
                                }),
                            )
                            .track_scroll(&self.connection_scroll[index])
                            .h(px(82.)),
                        ),
                )
            })
            .when_some(self.fields.get(index).cloned(), |body, field| {
                body.child(format!(
                    "Exact {} schema · 1–63 UTF-8 bytes",
                    side_label(side).to_lowercase()
                ))
                .child(field)
            })
            .child(self.schema_choices(side, cx))
            .into_any_element()
    }
    fn schema_choices(&self, side: Side, cx: &Context<Self>) -> AnyElement {
        let index = side_index(side);
        let count = self.cached_schemas(side).len();
        let setup_enabled = self.setup_enabled(cx);
        if count == 0 {
            return div()
                .child(
                    "No cached schema names. Enter the exact schema; no lookup runs while typing.",
                )
                .into_any_element();
        }
        let connection = match side {
            Side::Source => self.source.clone(),
            Side::Target => self.target.clone(),
        }
        .unwrap_or_default();
        let keyboard_connection = connection.clone();
        let generation = self.generation;
        div()
            .child("Cached schema names from open Objects tabs; exact text entry remains available")
            .child(
                div()
                    .id(("comparison-cached-schemas", index))
                    .role(Role::ListBox)
                    .a11y_synthetic_children(move |builder| {
                        if !setup_enabled {
                            builder.parent_node().set_disabled();
                        }
                    })
                    .aria_label(format!("{} cached schema names", side_label(side)))
                    .track_focus(&self.schema_focus[index])
                    .tab_index(0)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if !this.schema_focus[index].is_focused(window)
                            || this.composing(window, cx)
                        {
                            return;
                        }
                        let value = this
                            .fields
                            .get(index)
                            .and_then(|field| field.read(cx).value(cx).ok());
                        let schemas = this.cached_schemas(side);
                        let current = schemas
                            .iter()
                            .position(|schema| Some(schema.as_str()) == value.as_deref());
                        if let Some(next) = navigation(&event.keystroke.key, current, schemas.len())
                        {
                            let schema = schemas[next].clone();
                            this.select_schema(
                                side,
                                &keyboard_connection,
                                schema,
                                generation,
                                window,
                                cx,
                            );
                            this.schema_scroll[index]
                                .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        gpui::uniform_list(
                            ("comparison-schema-items", index),
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|row| {
                                        let Some(schema) =
                                            this.cached_schemas(side).get(row).cloned()
                                        else {
                                            return div().into_any_element();
                                        };
                                        let current = this
                                            .fields
                                            .get(index)
                                            .and_then(|field| field.read(cx).value(cx).ok());
                                        let selected = current.as_deref() == Some(schema.as_str());
                                        let accessible_schema = schema.clone();
                                        let label = schema.clone();
                                        let click_connection = connection.clone();
                                        let accessible_connection = connection.clone();
                                        let weak = cx.weak_entity();
                                        div()
                                            .id(("comparison-cached-schema", row))
                                            .role(Role::ListBoxOption)
                                            .a11y_synthetic_children(move |builder| {
                                                if !setup_enabled {
                                                    builder.parent_node().set_disabled();
                                                }
                                            })
                                            .aria_label(label.clone())
                                            .aria_selected(selected)
                                            .h(px(26.))
                                            .px_2()
                                            .when(selected, |row| row.bg(crate::style::select()))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.select_schema(
                                                    side,
                                                    &click_connection,
                                                    schema.clone(),
                                                    generation,
                                                    window,
                                                    cx,
                                                );
                                                window.focus(&this.schema_focus[index], cx);
                                            }))
                                            .on_a11y_action(
                                                gpui::accesskit::Action::Click,
                                                move |_, window, cx| {
                                                    weak.update(cx, |this, cx| {
                                                        this.select_schema(
                                                            side,
                                                            &accessible_connection,
                                                            accessible_schema.clone(),
                                                            generation,
                                                            window,
                                                            cx,
                                                        );
                                                        window.focus(&this.schema_focus[index], cx);
                                                    })
                                                    .ok();
                                                },
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.schema_scroll[index])
                        .h(px(78.)),
                    ),
            )
            .into_any_element()
    }
    fn choose_object(
        &mut self,
        request: &ResultRequest,
        object: RelationIdentity,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || self.composing(window, cx)
            || self
                .reader
                .read(cx)
                .state()
                .and_then(model::ReaderState::request)
                != Some(request)
        {
            return;
        }
        self.reader.update(cx, |reader, cx| {
            reader.enqueue(Intent::SelectObject(object), cx)
        });
        window.focus(&self.objects_focus, cx);
    }
    fn choose_field(
        &mut self,
        request: &ResultRequest,
        object: &RelationIdentity,
        path: FieldPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.editable
            || self.composing(window, cx)
            || self
                .reader
                .read(cx)
                .state()
                .and_then(model::ReaderState::request)
                != Some(request)
            || self
                .reader
                .read(cx)
                .state()
                .and_then(model::ReaderState::selected_object)
                != Some(object)
        {
            return;
        }
        self.reader.update(cx, |reader, cx| {
            reader.enqueue(Intent::SelectField(path), cx)
        });
        window.focus(&self.fields_focus, cx);
    }
    fn objects_element(&self, cx: &Context<Self>) -> AnyElement {
        let Some(state) = self.reader.read(cx).state() else {
            return div().into_any_element();
        };
        let Some(request) = state.request().cloned() else {
            return div().into_any_element();
        };
        let Some(CompareReply::Objects {
            offset,
            next_offset,
            items,
        }) = state.objects()
        else {
            return div()
                .child("Object page unread or unavailable.")
                .into_any_element();
        };
        let count = items.len();
        let header = format!(
            "Objects {}–{}{}",
            if count == 0 { 0 } else { *offset as usize + 1 },
            *offset as usize + count,
            if next_offset.is_some() {
                " · more available"
            } else {
                " · final page"
            }
        );
        let keyboard_request = request.clone();
        div()
            .flex_1()
            .min_w_0()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children([8, 9].map(|index| self.button(index, cx))),
            )
            .child(
                div()
                    .id("comparison-objects")
                    .role(Role::ListBox)
                    .aria_label("Comparison objects in the captured schemas")
                    .track_focus(&self.objects_focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if !this.objects_focus.is_focused(window) {
                            return;
                        }
                        let Some(state) = this.reader.read(cx).state() else {
                            return;
                        };
                        let Some(CompareReply::Objects { items, .. }) = state.objects() else {
                            return;
                        };
                        let current = items.iter().position(|item| {
                            Some(model::object_identity(item)) == state.selected_object()
                        });
                        if let Some(next) = navigation(&event.keystroke.key, current, items.len()) {
                            let object = model::object_identity(&items[next]).clone();
                            this.choose_object(&keyboard_request, object, window, cx);
                            this.object_scroll
                                .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                            cx.stop_propagation();
                        }
                    }))
                    .when(count == 0, |body| {
                        body.child("No objects on this page. Review coverage and exclusions.")
                    })
                    .child(
                        gpui::uniform_list(
                            "comparison-object-items",
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        let Some(state) = this
                                            .reader
                                            .read(cx)
                                            .state()
                                            .filter(|state| state.request() == Some(&request))
                                        else {
                                            return div().into_any_element();
                                        };
                                        let Some(CompareReply::Objects { items, .. }) =
                                            state.objects()
                                        else {
                                            return div().into_any_element();
                                        };
                                        let Some(item) = items.get(index) else {
                                            return div().into_any_element();
                                        };
                                        let object = model::object_identity(item).clone();
                                        let label = format!(
                                            "{} · {:?} · {} · {} changed / {} incomparable",
                                            object.name,
                                            object.kind,
                                            model::difference_label(&item.difference),
                                            item.changed_fields,
                                            item.incomparable_fields
                                        );
                                        let selected = state.selected_object() == Some(&object);
                                        let weak = cx.weak_entity();
                                        let click_request = request.clone();
                                        let accessible_request = request.clone();
                                        let accessible_object = object.clone();
                                        div()
                                            .id(("comparison-object", index))
                                            .role(Role::ListBoxOption)
                                            .aria_label(label.clone())
                                            .aria_selected(selected)
                                            .h(px(28.))
                                            .px_2()
                                            .when(selected, |row| row.bg(crate::style::select()))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.choose_object(
                                                    &click_request,
                                                    object.clone(),
                                                    window,
                                                    cx,
                                                )
                                            }))
                                            .on_a11y_action(
                                                gpui::accesskit::Action::Click,
                                                move |_, window, cx| {
                                                    weak.update(cx, |this, cx| {
                                                        this.choose_object(
                                                            &accessible_request,
                                                            accessible_object.clone(),
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                    .ok();
                                                },
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.object_scroll)
                        .h(px(190.)),
                    ),
            )
            .into_any_element()
    }
    fn fields_element(&self, cx: &Context<Self>) -> AnyElement {
        let Some(state) = self.reader.read(cx).state() else {
            return div().into_any_element();
        };
        let Some(request) = state.request().cloned() else {
            return div().into_any_element();
        };
        let Some(CompareReply::Fields {
            offset,
            next_offset,
            items,
            object,
        }) = state.fields()
        else {
            return div()
                .flex_1()
                .child(if state.selected_object_summary().is_some_and(|object| object.field_count == 0) { "No field page for this object. See the scope and eligibility details below." } else if state.selected_object().is_some() { "Fields not loaded. Retry the result read if unavailable." } else { "Select an object to inspect fields." })
                .into_any_element();
        };
        let count = items.len();
        let header = format!(
            "{} · Fields {}–{}{}",
            object.name,
            if count == 0 { 0 } else { *offset as usize + 1 },
            *offset as usize + count,
            if next_offset.is_some() {
                " · more available"
            } else {
                " · final page"
            }
        );
        let keyboard_request = request.clone();
        let captured_object = object.clone();
        let keyboard_object = object.clone();
        div()
            .flex_1()
            .min_w_0()
            .child(header)
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .children([10, 11].map(|index| self.button(index, cx))),
            )
            .child(
                div()
                    .id("comparison-fields")
                    .role(Role::ListBox)
                    .aria_label("Fields of selected comparison object")
                    .track_focus(&self.fields_focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if !this.fields_focus.is_focused(window) {
                            return;
                        }
                        let Some(state) = this.reader.read(cx).state() else {
                            return;
                        };
                        let Some(CompareReply::Fields { items, .. }) = state.fields() else {
                            return;
                        };
                        let current = items
                            .iter()
                            .position(|item| Some(&item.path) == state.selected_field());
                        if let Some(next) = navigation(&event.keystroke.key, current, items.len()) {
                            let path = items[next].path.clone();
                            this.choose_field(
                                &keyboard_request,
                                &keyboard_object,
                                path,
                                window,
                                cx,
                            );
                            this.field_scroll
                                .scroll_to_item(next, gpui::ScrollStrategy::Nearest);
                            cx.stop_propagation();
                        }
                    }))
                    .when(count == 0, |body| body.child("No fields on this page."))
                    .child(
                        gpui::uniform_list(
                            "comparison-field-items",
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        let Some(state) = this
                                            .reader
                                            .read(cx)
                                            .state()
                                            .filter(|state| state.request() == Some(&request))
                                        else {
                                            return div().into_any_element();
                                        };
                                        let Some(CompareReply::Fields { items, .. }) =
                                            state.fields()
                                        else {
                                            return div().into_any_element();
                                        };
                                        let Some(item) = items.get(index) else {
                                            return div().into_any_element();
                                        };
                                        let path = item.path.clone();
                                        let label = format!(
                                            "{} · Source: {} · Target: {} · {}{}",
                                            model::field_label(&path),
                                            model::field_side_label(state, item, Side::Source),
                                            model::field_side_label(state, item, Side::Target),
                                            model::difference_label(&item.difference),
                                            match &item.difference {
                                                SummaryDifference::NotComparable {
                                                    reason, ..
                                                } => format!(
                                                    " · {}",
                                                    model::incomparable_label(*reason)
                                                ),
                                                _ => String::new(),
                                            }
                                        );
                                        let selected = state.selected_field() == Some(&path);
                                        let weak = cx.weak_entity();
                                        let click_request = request.clone();
                                        let accessible_request = request.clone();
                                        let accessible_path = path.clone();
                                        let click_object = captured_object.clone();
                                        let accessible_object = captured_object.clone();
                                        div()
                                            .id(("comparison-field", index))
                                            .role(Role::ListBoxOption)
                                            .aria_label(label.clone())
                                            .aria_selected(selected)
                                            .h(px(28.))
                                            .px_2()
                                            .when(selected, |row| row.bg(crate::style::select()))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.choose_field(
                                                    &click_request,
                                                    &click_object,
                                                    path.clone(),
                                                    window,
                                                    cx,
                                                )
                                            }))
                                            .on_a11y_action(
                                                gpui::accesskit::Action::Click,
                                                move |_, window, cx| {
                                                    weak.update(cx, |this, cx| {
                                                        this.choose_field(
                                                            &accessible_request,
                                                            &accessible_object,
                                                            accessible_path.clone(),
                                                            window,
                                                            cx,
                                                        )
                                                    })
                                                    .ok();
                                                },
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.field_scroll)
                        .h(px(190.)),
                    ),
            )
            .into_any_element()
    }
    fn value_element(&self, side: Side, cx: &Context<Self>) -> AnyElement {
        let Some(state) = self.reader.read(cx).state() else {
            return div().into_any_element();
        };
        if state.selected_object().is_none() {
            return div().into_any_element();
        }
        let index = side_index(side);
        let text = match model::value_state(state, side) {
            model::ValueState::Absent => "Absent on this side".into(),
            model::ValueState::Excluded(reason) => format!(
                "Excluded from comparison: {}",
                reason.map_or_else(
                    || "outside supported scope".into(),
                    |reason| format!("{reason:?}")
                )
            ),
            model::ValueState::Null => "SQL NULL".into(),
            model::ValueState::Empty => "Empty text (0 bytes)".into(),
            model::ValueState::Unread => {
                "Unread or unavailable. Select a field, or explicitly retry the result read.".into()
            }
            model::ValueState::Text {
                value,
                offset,
                next,
                total,
                complete,
            } => format!(
                "Bytes {offset}–{next} of {total}{}\n{value}",
                if complete {
                    " · final chunk"
                } else {
                    " · more available"
                }
            ),
        };
        div()
            .flex_1()
            .min_w_0()
            .child(state.selected_value(side).map_or_else(
                || side_label(side).to_owned(),
                |value| {
                    format!(
                        "{} · {} · {} B",
                        side_label(side),
                        model::value_kind_label(value.value_kind),
                        value.raw_bytes
                    )
                },
            ))
            .child(crate::ui::toolbar().children(
                [12 + index * 2, 13 + index * 2, 16 + index].map(|index| self.button(index, cx)),
            ))
            .child(
                div()
                    .id(("comparison-value", index))
                    .role(Role::Label)
                    .aria_label(format!("{} value: {text}", side_label(side)))
                    .track_focus(&self.value_focus[index])
                    .tab_index(0)
                    .p_2()
                    .whitespace_normal()
                    .child(text),
            )
            .into_any_element()
    }
    fn jobs_element(&self, cx: &Context<Self>) -> AnyElement {
        let rows = self
            .store
            .read(cx)
            .capture()
            .map_or(&[][..], |capture| capture.rows());
        div().child("Session comparisons · up to 2 active and 2 terminal jobs; completed results expire")
            .child(crate::ui::toolbar().children([1,2,3,4].map(|index|self.button(index,cx))))
            .child(div().id("comparison-jobs").role(Role::ListBox).aria_label("Session schema comparison jobs").track_focus(&self.jobs_focus).tab_index(0)
                .on_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{if !this.jobs_focus.is_focused(window){return;}let rows=this.store.read(cx).capture().map_or(&[][..],|capture|capture.rows());let current=rows.iter().position(|job|Some(job.job_id.as_str())==this.selected.as_deref());if let Some(next)=navigation(&event.keystroke.key,current,rows.len()){let id=rows[next].job_id.clone();this.select_job(id,window,cx);cx.stop_propagation();}}))
                .when(rows.is_empty(),|body|body.child("No comparison jobs observed. A missing observation does not prove a queued admission failed."))
                .children(rows.iter().enumerate().map(|(index,job)|{
                    let label=format!("{} · {} → {} · {} source / {} target objects{}",model::phase_label(&job.state),self.endpoint_label(&job.source),self.endpoint_label(&job.target),job.source_objects,job.target_objects,match &job.state{StatusState::Failed{failure}=>format!(" · {}",model::job_failure_text(failure)),_=>String::new()});let selected=self.selected.as_deref()==Some(job.job_id.as_str());let id=job.job_id.clone();let accessible_id=id.clone();let weak=cx.weak_entity();
                    div().id(("comparison-job",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).p_2().when(selected,|row|row.bg(crate::style::select())).child(label)
                        .on_click(cx.listener(move|this,_,window,cx|this.select_job(id.clone(),window,cx)))
                        .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|this.select_job(accessible_id.clone(),window,cx)).ok();})
                })))
            .when(self.selected.is_some()&&self.selected_job(cx).is_none(),|body|body.child("Selected job is missing or expired. No other job was selected."))
            .child("Accepted endpoints stay fixed when setup changes. Closing this tab keeps accepted jobs alive. Cancel waits for owned work; no schema writes are performed.").into_any_element()
    }
}
impl Focusable for SchemaCompareView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for SchemaCompareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare_fields(window, cx);
        let reader = self.reader.read(cx);
        let reader_message = reader.message().map(str::to_owned);
        let store_message = self.store.read(cx).message().map(str::to_owned);
        let coverage = reader
            .state()
            .and_then(model::ReaderState::metadata)
            .and_then(model::coverage_text);
        let summary = reader
            .state()
            .and_then(model::ReaderState::metadata)
            .and_then(model::summary_text);
        let object_detail = reader.state().and_then(model::object_detail);
        let accepted = self.selected_job(cx).map(|job| {
            let differs = self.endpoint(Side::Source, cx).as_ref() != Ok(&job.source)
                || self.endpoint(Side::Target, cx).as_ref() != Ok(&job.target);
            format!(
                "Accepted endpoints: {} → {}{}",
                self.endpoint_label(&job.source),
                self.endpoint_label(&job.target),
                if differs {
                    "
Draft endpoints differ from this comparison."
                } else {
                    ""
                }
            )
        });
        let detail = reader.state().and_then(|state| {
            let item = state.selected_field_summary()?;
            Some(format!(
                "Selected field: {} · {}\nSource: {}\nTarget: {}{}",
                model::field_label(&item.path),
                model::difference_label(&item.difference),
                model::field_side_label(state, item, Side::Source),
                model::field_side_label(state, item, Side::Target),
                match &item.difference {
                    SummaryDifference::NotComparable { reason, .. } =>
                        format!("\n{}", model::incomparable_label(*reason)),
                    _ => String::new(),
                }
            ))
        });
        div()
            .id("schema-comparison-view")
            .key_context("SchemaComparison")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_size(px(crate::style::FONT))
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.composing(window, cx) {
                    return;
                }
                if event.keystroke.key == "c"
                    && event.keystroke.modifiers.platform
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && this.copy_focused(window, cx)
                {
                    cx.stop_propagation();
                    window.prevent_default();
                    return;
                }
                if event.keystroke.key == "tab"
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                    && !event.keystroke.modifiers.platform
                {
                    let order = this.focus_order(cx);
                    if !order.is_empty() {
                        let current = order.iter().position(|focus| focus.is_focused(window));
                        let next = if event.keystroke.modifiers.shift {
                            current.map_or(order.len() - 1, |index| {
                                (index + order.len() - 1) % order.len()
                            })
                        } else {
                            current.map_or(0, |index| (index + 1) % order.len())
                        };
                        window.focus(&order[next], cx);
                        cx.stop_propagation();
                        window.prevent_default();
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &crate::grid::CopyCells, window, cx| {
                if this.copy_focused(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(
                crate::ui::toolbar()
                    .child(crate::ui::section_label("Schema comparison"))
                    .child(crate::ui::badge("read-only"))
                    .child("PostgreSQL 16 ordinary tables"),
            )
            .child(
                div()
                    .id("comparison-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .p_2()
                            .child(self.connection_element(Side::Source, cx))
                            .child(self.connection_element(Side::Target, cx)),
                    )
                    .child(
                        crate::ui::toolbar()
                            .children([0, 5, 6, 7].map(|index| self.button(index, cx))),
                    )
                    .child(
                        div()
                            .id("comparison-status")
                            .role(Role::Label)
                            .aria_label(self.status.clone())
                            .p_2()
                            .text_color(crate::style::dim())
                            .child(self.status.clone()),
                    )
                    .when_some(store_message, |body, message| {
                        body.child(div().p_2().child(crate::ui::shake(
                            format!("comparison-store-error-shake-{message}"),
                            crate::ui::error_banner("comparison-store-error", message),
                        )))
                    })
                    .when_some(reader_message, |body, message| {
                        body.child(div().p_2().child(crate::ui::shake(
                            format!("comparison-reader-error-shake-{message}"),
                            crate::ui::error_banner("comparison-reader-error", message),
                        )))
                    })
                    .when_some(accepted, |body, text| {
                        body.child(
                            div()
                                .id("comparison-accepted-endpoints")
                                .role(Role::Label)
                                .aria_label(text.clone())
                                .p_2()
                                .whitespace_normal()
                                .child(text),
                        )
                    })
                    .when_some(summary, |body, text| {
                        body.child(
                            div()
                                .id("comparison-result-summary")
                                .role(Role::Label)
                                .aria_label(text.clone())
                                .p_2()
                                .child(text),
                        )
                    })
                    .when(self.coverage, |body| {
                        body.when_some(coverage, |body, text| {
                            body.child(
                                div()
                                    .id("comparison-coverage")
                                    .role(Role::Label)
                                    .aria_label(text.clone())
                                    .track_focus(&self.coverage_focus)
                                    .tab_index(0)
                                    .p_2()
                                    .whitespace_normal()
                                    .child(text),
                            )
                        })
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .p_2()
                            .child(self.objects_element(cx))
                            .child(self.fields_element(cx)),
                    )
                    .when_some(object_detail, |body, text| {
                        body.child(
                            div()
                                .id("comparison-selected-object")
                                .role(Role::Label)
                                .aria_label(text.clone())
                                .p_2()
                                .whitespace_normal()
                                .child(text),
                        )
                    })
                    .when_some(detail, |body, text| {
                        body.child(
                            div()
                                .id("comparison-selected-field")
                                .role(Role::Label)
                                .aria_label(text.clone())
                                .p_2()
                                .whitespace_normal()
                                .child(text),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .p_2()
                            .child(self.value_element(Side::Source, cx))
                            .child(self.value_element(Side::Target, cx)),
                    )
                    .child(self.jobs_element(cx)),
            )
    }
}

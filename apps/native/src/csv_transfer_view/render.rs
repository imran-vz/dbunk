use super::*;
fn preview(value: &str) -> String {
    let mut chars = value.chars();
    let text = chars.by_ref().take(256).collect::<String>();
    if chars.next().is_some() {
        format!("{text}… (select to inspect)")
    } else {
        text
    }
}
impl CsvTransferView {
    // Page keys expose the complete review/receipt without stealing editor or list keys.
    fn scroll_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control
            || modifiers.alt
            || modifiers.platform
            || modifiers.shift
            || self.composing(window, cx)
            || self
                .fields
                .iter()
                .any(|field| field.focus_handle(cx).contains_focused(window, cx))
        {
            return false;
        }
        let mut offset = self.scroll.offset();
        let bottom = -self.scroll.max_offset().y;
        let details = self.details.is_focused(window);
        offset.y = match event.keystroke.key.as_str() {
            "pageup" => offset.y + px(160.),
            "pagedown" => offset.y - px(160.),
            "up" if details => offset.y + px(28.),
            "down" if details => offset.y - px(28.),
            "home" if details => px(0.),
            "end" if details => bottom,
            _ => return false,
        }
        .max(bottom)
        .min(px(0.));
        self.scroll.set_offset(offset);
        cx.notify();
        cx.stop_propagation();
        window.prevent_default();
        true
    }
    pub(super) fn button(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let (action, default) = ACTIONS[index];
        let enabled = self.enabled(action, cx);
        let selected = self.setup.as_ref().is_some_and(|setup| match action {
            Action::Import => setup.direction() == CsvDirection::Import && !setup.xlsx(),
            Action::Xlsx => setup.xlsx(),
            Action::Export => setup.direction() == CsvDirection::Export,
            Action::Header => setup.options().header,
            Action::Delimiter(value) => self
                .fields
                .get(2)
                .is_some_and(|field| field.read(cx).value(cx).is_ok_and(|text| text == value)),
            Action::Schemas => !self.metadata.tables,
            Action::Tables => self.metadata.tables,
            _ => false,
        });
        let toggle = matches!(
            action,
            Action::Import
                | Action::Xlsx
                | Action::Export
                | Action::Header
                | Action::Schemas
                | Action::Tables
                | Action::Delimiter(_)
        );
        let unknown = matches!(action, Action::Dismiss)
            && self.selected_row(cx).is_some_and(|row| {
                row.direction == CsvDirection::Import && row.effect == CsvEffect::Unknown
            });
        let label =
            if matches!(action, Action::Pick) && self.setup.as_ref().is_some_and(Setup::xlsx) {
                "Choose XLSX file"
            } else if unknown {
                if self.unknown_ack == self.selected {
                    "I inspected the target; dismiss unknown outcome"
                } else {
                    "Inspect target before dismissing unknown outcome"
                }
            } else {
                default
            };
        let weak = cx.weak_entity();
        div()
            .id(("csv-action", index))
            .role(if toggle { Role::CheckBox } else { Role::Button })
            .aria_label(label)
            .a11y_synthetic_children(move |builder| {
                if !enabled {
                    builder.parent_node().set_disabled();
                }
                if toggle {
                    builder
                        .parent_node()
                        .set_toggled(gpui::accesskit::Toggled::from(selected));
                }
            })
            .track_focus(&self.buttons[index])
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .text_color(if enabled {
                crate::style::text()
            } else {
                crate::style::dim()
            })
            .when(selected, |element| element.bg(crate::style::select()))
            .focus(|style| style.bg(crate::style::line()))
            .child(label)
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                weak.update(cx, |this, cx| this.activate(action, window, cx))
                    .ok();
            })
            .into_any_element()
    }
    fn review_text(&self, cx: &gpui::App) -> Option<String> {
        let review = self.current_review(cx)?;
        let data = review.inspection().data();
        let target = review.target();
        let provenance = data
            .workbook
            .as_ref()
            .map_or_else(String::new, model::workbook_summary);
        Some(format!(
            "Exact reviewed target\nConnection: {}\nHost: {}\nPort: {}\nDatabase: {}\nUser: {}\nEnvironment: {}\nSafe mode: {}\nRead-only: {}\nRelation: {}.{}\nDirection: {}\nMapped columns: {}\nInspection expires: {}\nSelected file name: {}\nObserved source bytes: {}\nUTF-8 · Header: {}\nDelimiter: {:?} · Quote: {:?} · Escape: {:?}\nNULL token (exact text): {:?}\n{}\n{}",
            target.connection_name,
            target.host,
            target.port,
            target.database,
            target.user,
            target.environment,
            target.safe_mode,
            target.read_only,
            data.target.schema,
            data.target.table,
            if data.workbook.is_some() {
                "Import XLSX"
            } else {
                model::direction_label(data.direction)
            },
            review.mapping().len(),
            data.expires_at,
            review.file_name(),
            data.total_bytes
                .map_or_else(|| "Unavailable".into(), |bytes| bytes.to_string()),
            data.options.header,
            data.options.delimiter,
            data.options.quote,
            data.options.escape,
            data.options.null_token,
            provenance,
            if data.workbook.is_some() {
                "Append only, one transaction, first error aborts. Defaults, constraints and triggers run. Sequence increments and external trigger effects can survive rollback."
            } else if data.direction == CsvDirection::Import {
                "Append only, one transaction, first error aborts. Defaults, constraints and triggers run. Sequence increments and external trigger effects can survive rollback. Source must remain unchanged through completion. ISO dates and UTC."
            } else {
                "Whole committed relation, including partitions. Grid filters, selections and staged changes are excluded. Row order is unspecified. New destination only; an unfinished partial file is not a completed export."
            }
        ))
    }
    fn mapping_element(&self, cx: &Context<Self>) -> AnyElement {
        let Some(data) = self.inspection_data(cx) else {
            return div().into_any_element();
        };
        let id = data.inspection_id;
        let count = data.source_columns.len();
        let error = self
            .mapping
            .as_ref()
            .and_then(|mapping| mapping.validate(data).err());
        let missing = self
            .mapping
            .as_ref()
            .and_then(|mapping| mapping.first_missing_required(data))
            .map(str::to_owned);
        let selected_text = self
            .mapping_source
            .and_then(|index| data.source_columns.get(index))
            .map(|column| {
                format!(
                    "Source {}: {}",
                    column.index + 1,
                    if column.name.is_empty() {
                        "(blank)"
                    } else {
                        &column.name
                    }
                )
            });
        div().child("Source columns are indexed; duplicate and blank headers remain distinct. Unmapped targets use database defaults.")
            .when_some(error,|body,error|body.child(div().id("csv-mapping-error").role(Role::Alert).child(error)))
            .when_some(missing,|body,name|body.child(format!("Required target: {name}")))
            .child(div().flex().flex_wrap().children((19..22).map(|index|self.button(index,cx))))
            .child(div().id("csv-mapping-list").role(Role::ListBox).aria_label("CSV source to target column mapping").track_focus(&self.mapping_focus).tab_index(0)
                .on_key_down(cx.listener(move|this,event:&KeyDownEvent,window,cx|{if !this.mapping_focus.is_focused(window)||this.inspection_data(cx).is_none_or(|data|data.inspection_id!=id){return;}let next=match event.keystroke.key.as_str(){"up"=>this.mapping_source.unwrap_or(0).saturating_sub(1),"down"=>this.mapping_source.map_or(0,|index|(index+1).min(count.saturating_sub(1))),"home"=>0,"end"=>count.saturating_sub(1),_=>return};this.mapping_source=(count>0).then_some(next);cx.notify();cx.stop_propagation();}))
                .child(gpui::uniform_list("csv-mapping",count,cx.processor(move|this,range:std::ops::Range<usize>,_,cx|{range.map(|index|{
                    let Some(data)=this.inspection_data(cx).filter(|data|data.inspection_id==id)else{return div().into_any_element();};let Some(source)=data.source_columns.get(index)else{return div().into_any_element();};
                    let target=this.mapping.as_ref().and_then(|mapping|mapping.target_index(index)).and_then(|index|data.target_columns.get(index));
                    let label=format!("{}: {} → {}",index+1,if source.name.is_empty(){"(blank)".into()}else{preview(&source.name)},target.map_or_else(||"Skip column".into(),|column|format!("{} ({})",column.name,preview(&column.data_type))));let selected=this.mapping_source==Some(index);let weak=cx.weak_entity();
                    div().id(("csv-source-column",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).h(px(28.)).px_2().when(selected,|row|row.bg(crate::style::select())).child(label)
                        .on_click(cx.listener(move|this,_,window,cx|{if this.inspection_data(cx).is_some_and(|data|data.inspection_id==id){this.mapping_source=Some(index);window.focus(&this.mapping_focus,cx);cx.notify();}}))
                        .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|{if this.inspection_data(cx).is_some_and(|data|data.inspection_id==id){this.mapping_source=Some(index);window.focus(&this.mapping_focus,cx);cx.notify();}}).ok();}).into_any_element()
                }).collect()})).h(px(140.))))
            .when_some(selected_text,|body,text|body.child(div().id("csv-source-full-name").role(Role::Label).aria_label(text.clone()).whitespace_normal().child(text))).into_any_element()
    }
    fn review_mapping_element(&self, cx: &Context<Self>) -> AnyElement {
        let Some(review) = self.current_review(cx) else {
            return div().into_any_element();
        };
        let id = review.inspection().inspection_id();
        let attempt = review.attempt_id();
        let count = review.mapping().len();
        if count == 0 {
            return div().into_any_element();
        }
        let detail = self.review_pair.and_then(|index| {
            let pair = review.mapping().get(index)?;
            let source = review
                .inspection()
                .data()
                .source_columns
                .get(pair.source_index)?;
            Some(format!(
                "Reviewed source {}: {}\nExact target column: {}",
                source.index + 1,
                source.name,
                pair.target_column
            ))
        });
        div().child("Exact reviewed column mapping · Unlisted source columns are skipped; unlisted targets use database defaults")
            .child(div().id("csv-reviewed-mapping-list").role(Role::ListBox).aria_label("Immutable reviewed CSV mapping").track_focus(&self.review_focus).tab_index(0)
                .on_key_down(cx.listener(move|this,event:&KeyDownEvent,window,cx|{if !this.review_focus.is_focused(window)||this.current_review(cx).is_none_or(|review|review.inspection().inspection_id()!=id||review.attempt_id()!=attempt){return;}let next=match event.keystroke.key.as_str(){"up"=>this.review_pair.unwrap_or(0).saturating_sub(1),"down"=>this.review_pair.map_or(0,|index|(index+1).min(count.saturating_sub(1))),"home"=>0,"end"=>count.saturating_sub(1),_=>return};this.review_pair=Some(next);cx.notify();cx.stop_propagation();}))
                .child(gpui::uniform_list("csv-reviewed-mapping",count,cx.processor(move|this,range:std::ops::Range<usize>,_,cx|{range.map(|index|{
                    let Some(review)=this.current_review(cx).filter(|review|review.inspection().inspection_id()==id&&review.attempt_id()==attempt)else{return div().into_any_element();};
                    let Some(pair)=review.mapping().get(index)else{return div().into_any_element();};let source=review.inspection().data().source_columns.get(pair.source_index).map_or("Unavailable",|column|column.name.as_str());
                    let label=format!("{}: {} → {}",pair.source_index+1,preview(source),pair.target_column);let selected=this.review_pair==Some(index);let weak=cx.weak_entity();
                    div().id(("csv-reviewed-pair",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).h(px(28.)).px_2().when(selected,|row|row.bg(crate::style::select())).child(label)
                        .on_click(cx.listener(move|this,_,window,cx|{if this.current_review(cx).is_some_and(|review|review.inspection().inspection_id()==id&&review.attempt_id()==attempt){this.review_pair=Some(index);window.focus(&this.review_focus,cx);cx.notify();}}))
                        .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|{if this.current_review(cx).is_some_and(|review|review.inspection().inspection_id()==id&&review.attempt_id()==attempt){this.review_pair=Some(index);window.focus(&this.review_focus,cx);cx.notify();}}).ok();}).into_any_element()
                }).collect()})).h(px(140.))))
            .when_some(detail,|body,detail|body.child(div().id("csv-reviewed-pair-detail").role(Role::Label).aria_label(detail.clone()).whitespace_normal().child(detail))).into_any_element()
    }
    fn sample_element(&self, cx: &Context<Self>) -> AnyElement {
        let Some(data) = self.inspection_data(cx) else {
            return div().into_any_element();
        };
        let id = data.inspection_id;
        let columns = data.source_columns.len();
        let count = data.sample_rows.len() * columns;
        let disclosure = format!(
            "Preview: {} records · {}. Total rows unknown. Inspection scans at most 256 KiB and 50 records, with 64 KiB of sample values. NULL differs from empty text and quoted NULL tokens.",
            data.sample_rows.len(),
            if data.sample_truncated {
                "truncated"
            } else {
                "within inspection limits"
            }
        );
        let detail = self.sample_cell.and_then(|index| {
            if columns == 0 {
                return None;
            }
            let value = data
                .sample_rows
                .get(index / columns)?
                .get(index % columns)?;
            Some(format!(
                "Record {}, source column {}\n{}",
                index / columns + 1,
                index % columns + 1,
                value.as_ref().map_or("SQL NULL", String::as_str)
            ))
        });
        div()
            .child(disclosure)
            .child(
                div()
                    .id("csv-sample-list")
                    .role(Role::ListBox)
                    .aria_label("CSV preview cells by record and source index")
                    .track_focus(&self.sample_focus)
                    .tab_index(0)
                    .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                        if !this.sample_focus.is_focused(window)
                            || this
                                .inspection_data(cx)
                                .is_none_or(|data| data.inspection_id != id)
                        {
                            return;
                        }
                        let next = match event.keystroke.key.as_str() {
                            "up" => this.sample_cell.unwrap_or(0).saturating_sub(1),
                            "down" => this
                                .sample_cell
                                .map_or(0, |index| (index + 1).min(count.saturating_sub(1))),
                            "home" => 0,
                            "end" => count.saturating_sub(1),
                            _ => return,
                        };
                        this.sample_cell = (count > 0).then_some(next);
                        cx.notify();
                        cx.stop_propagation();
                    }))
                    .child(
                        gpui::uniform_list(
                            "csv-sample",
                            count,
                            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|index| {
                                        let Some(data) = this
                                            .inspection_data(cx)
                                            .filter(|data| data.inspection_id == id)
                                        else {
                                            return div().into_any_element();
                                        };
                                        if columns == 0 {
                                            return div().into_any_element();
                                        }
                                        let Some(value) = data
                                            .sample_rows
                                            .get(index / columns)
                                            .and_then(|row| row.get(index % columns))
                                        else {
                                            return div().into_any_element();
                                        };
                                        let label = format!(
                                            "Record {}, column {}: {}",
                                            index / columns + 1,
                                            index % columns + 1,
                                            value.as_ref().map_or_else(
                                                || "NULL".into(),
                                                |value| if value.is_empty() {
                                                    "Empty text".into()
                                                } else {
                                                    preview(value)
                                                }
                                            )
                                        );
                                        let selected = this.sample_cell == Some(index);
                                        let weak = cx.weak_entity();
                                        div()
                                            .id(("csv-sample-cell", index))
                                            .role(Role::ListBoxOption)
                                            .aria_label(label.clone())
                                            .aria_selected(selected)
                                            .h(px(28.))
                                            .px_2()
                                            .when(selected, |row| row.bg(crate::style::select()))
                                            .child(label)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                if this
                                                    .inspection_data(cx)
                                                    .is_some_and(|data| data.inspection_id == id)
                                                {
                                                    this.sample_cell = Some(index);
                                                    window.focus(&this.sample_focus, cx);
                                                    cx.notify();
                                                }
                                            }))
                                            .on_a11y_action(
                                                gpui::accesskit::Action::Click,
                                                move |_, window, cx| {
                                                    weak.update(cx, |this, cx| {
                                                        if this.inspection_data(cx).is_some_and(
                                                            |data| data.inspection_id == id,
                                                        ) {
                                                            this.sample_cell = Some(index);
                                                            window.focus(&this.sample_focus, cx);
                                                            cx.notify();
                                                        }
                                                    })
                                                    .ok();
                                                },
                                            )
                                            .into_any_element()
                                    })
                                    .collect()
                            }),
                        )
                        .h(px(140.)),
                    ),
            )
            .when_some(detail, |body, detail| {
                body.child(
                    div()
                        .id("csv-sample-detail")
                        .role(Role::Label)
                        .aria_label(detail.clone())
                        .whitespace_normal()
                        .child(detail),
                )
            })
            .into_any_element()
    }
}
impl Focusable for CsvTransferView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for CsvTransferView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rebuild_fields(window, cx);
        self.sync_inspection(cx);
        self.freeze_fields(cx);
        let connection = self
            .connection
            .clone()
            .unwrap_or_else(|| "Select a saved PostgreSQL connection".into());
        let filename = self
            .setup
            .as_ref()
            .and_then(Setup::path)
            .and_then(|path| path.file_name())
            .map_or_else(
                || "No file selected".into(),
                |name| name.to_string_lossy().into_owned(),
            );
        let store = self.store.read(cx);
        let message = store.message().map(str::to_owned);
        let limits = store.capture().map(|capture| capture.limits());
        let jobs = store.capture().map_or_else(Vec::new, |capture| {
            capture
                .jobs()
                .iter()
                .enumerate()
                .filter(|(_, row)| Some(row.connection_id.as_str()) == self.connection.as_deref())
                .filter_map(|(index, row)| Some((row.attempt_id, capture.row_label(index)?)))
                .collect::<Vec<_>>()
        });
        let details = self.selected.and_then(|key| {
            let capture = store.capture()?;
            capture.details(capture.index_for_key(key)?)
        });
        let inspection_status = self.inspection.as_ref().map(|(id, _)| {
            let Some(row) = store.capture().and_then(|capture| capture.inspection(*id)) else {
                return "Inspection admission/observation pending or unavailable. Missing is not proof of refusal.".into();
            };
            let detail = if let Some(diagnostic) = &row.diagnostic {
                format!(
                    " · {} · record {} · column {} · SQLSTATE {}",
                    diagnostic.reason,
                    diagnostic.record.map_or_else(|| "unavailable".into(), |value| value.to_string()),
                    diagnostic.column.map_or_else(|| "unavailable".into(), |value| value.to_string()),
                    diagnostic.sqlstate.as_deref().unwrap_or("unavailable"),
                )
            } else {
                row.failure.map_or_else(String::new, |error| format!(" · {error}"))
            };
            format!("Inspection: {:?} · Cleanup: {:?}{detail}", row.phase, row.cleanup)
        });
        let review = self.review_text(cx);
        let import = self
            .setup
            .as_ref()
            .is_none_or(|setup| setup.direction() == CsvDirection::Import);
        let xlsx = self.setup.as_ref().is_some_and(Setup::xlsx);
        let fields = self.fields.clone();
        let workbook = self.workbook_element(cx);
        let has_data = self.inspection_data(cx).is_some();
        div().id("csv-transfer-view").key_context("CsvTransfer").track_focus(&self.focus).size_full().flex().flex_col().bg(crate::style::bg()).text_color(crate::style::text()).text_size(px(12.))
            .capture_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{if this.composing(window,cx){return;}
                if event.keystroke.key=="tab" && !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt && !event.keystroke.modifiers.platform{let order=this.focus_order(cx);if !order.is_empty(){let current=order.iter().position(|focus|focus.is_focused(window));let next=if event.keystroke.modifiers.shift{current.map_or(order.len()-1,|index|(index+order.len()-1)%order.len())}else{current.map_or(0,|index|(index+1)%order.len())};window.focus(&order[next],cx);cx.stop_propagation();window.prevent_default();}}}))
            .on_key_down(cx.listener(|this,event:&KeyDownEvent,window,cx|{if this.scroll_key(event,window,cx){return;}
                if !this.list.is_focused(window){return;}let rows=this.store.read(cx).capture().map_or_else(Vec::new,|capture|capture.jobs().iter().filter(|row|Some(row.connection_id.as_str())==this.connection.as_deref()).map(|row|row.attempt_id).collect::<Vec<_>>());let current=rows.iter().position(|id|Some(*id)==this.selected);let next=match event.keystroke.key.as_str(){"up"=>current.unwrap_or(0).saturating_sub(1),"down"=>current.map_or(0,|index|(index+1).min(rows.len().saturating_sub(1))),"home"=>0,"end"=>rows.len().saturating_sub(1),"enter" if this.selected_row(cx).is_some()=>{window.focus(&this.details,cx);cx.stop_propagation();return;},_=>return};this.selected=rows.get(next).copied();this.unknown_ack=None;cx.notify();cx.stop_propagation();}))
            .child(div().flex().flex_wrap().p_2().child("Data transfer").children([0,27,1].map(|index|self.button(index,cx))))
            .child(div().id("csv-connection").role(Role::Label).aria_label(format!("Connection: {connection}")).px_2().child(connection))
            .child(div().id("csv-body").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.scroll)
                .children(fields.into_iter().enumerate().filter(|(index,_)| !xlsx || !(2..5).contains(index)).map(|(index,field)|div().px_2().child(["Schema","Table","Delimiter · one byte","Quote · one byte","Escape · one byte","NULL token · up to 64 Unicode characters"][index]).child(field)))
                .when(!xlsx,|body|body.child(div().flex().flex_wrap().p_2().child(self.button(2,cx)).children((23..27).map(|index|self.button(index,cx))).child("UTF-8")))
                .child(self.choices_element(cx)).child(div().id("csv-file").role(Role::Label).aria_label(format!("Selected file: {filename}")).p_2().child(filename))
                .child(if xlsx{"A private workbook snapshot supplies the selected sheet. Automatic headers; exact NULL-token matches become SQL NULL. Cached formula values only, no calculation. Dates remain Excel serial text. Oversized or unsupported workbooks are refused."}else if import{"Keep the source file unchanged through completion. Import appends in one transaction; defaults, constraints and triggers run. Unquoted NULL tokens become SQL NULL; quoted tokens remain text."}else{"Export the whole committed table including partitions. Existing destination files are never replaced. Grid filters, loaded rows and staged edits do not change export scope."})
                .child(div().flex().flex_wrap().p_2().children([3,4,5,22,13].map(|index|self.button(index,cx))).when(xlsx,|body|body.child(self.button(28,cx))))
                .child(workbook)
                .when_some(inspection_status,|body,status|body.child(div().id("csv-inspection-status").role(Role::Label).aria_label(status.clone()).child(status)))
                .when(import&&has_data,|body|body.child(self.mapping_element(cx)).child(self.sample_element(cx)))
                .child(div().flex().flex_wrap().p_2().children([6,7,8].map(|index|self.button(index,cx))))
                .when_some(review,|body,review|body.child(div().id("csv-review").role(Role::Label).aria_label(review.clone()).p_2().whitespace_normal().child(review)).child(self.review_mapping_element(cx)))
                .child(div().id("csv-status").role(Role::Label).aria_label(self.status.clone()).p_2().child(self.status.clone()))
                .when_some(message,|body,message|body.child(div().id("csv-store-message").role(Role::Alert).p_2().child(message)))
                .child(div().flex().flex_wrap().p_2().children((9..13).map(|index|self.button(index,cx))))
                .when_some(limits,|body,limits|body.child(limits))
                .child(div().id("csv-job-list").role(Role::ListBox).aria_label("CSV transfers in this session").track_focus(&self.list).tab_index(0)
                    .when(jobs.is_empty(),|body|body.child("No observed transfers for this connection."))
                    .children(jobs.into_iter().enumerate().map(|(index,(id,label))|{let selected=self.selected==Some(id);let weak=cx.weak_entity();div().id(("csv-job",index)).role(Role::ListBoxOption).aria_label(label.clone()).aria_selected(selected).px_2().py_1().when(selected,|row|row.bg(crate::style::select())).child(label)
                        .on_click(cx.listener(move|this,_,window,cx|{this.selected=Some(id);this.unknown_ack=None;window.focus(&this.list,cx);cx.notify();}))
                        .on_a11y_action(gpui::accesskit::Action::Click,move|_,window,cx|{weak.update(cx,|this,cx|{this.selected=Some(id);this.unknown_ack=None;window.focus(&this.list,cx);cx.notify();}).ok();})})))
                .when_some(details,|body,details|body.child(div().id("csv-job-details").role(Role::Label).aria_label(details.clone()).track_focus(&self.details).tab_index(0).p_2().whitespace_normal().child(details)))
                .when(self.selected.is_some()&&self.selected_row(cx).is_none(),|body|body.child("Selected transfer is missing or expired; no other transfer was selected. Refresh to reconcile."))
                .child(div().p_2().child("Closing setup releases its unused inspections; accepted jobs continue. Saving or disconnecting a connection can stop active work. Cancellation cannot promise rollback after commit/publication begins.")))
    }
}

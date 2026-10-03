use super::*;
use gpui::uniform_list;

impl PgToolView {
    pub(super) fn choices_element(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .metadata
            .capture
            .as_ref()
            .map_or(0, |capture| capture.visible.len());
        let truncated = self
            .metadata
            .capture
            .as_ref()
            .map_or(0, |capture| capture.catalog.truncated.len());
        let kind = if self.metadata.kind == choices::ChoiceKind::Schema {
            "Schemas"
        } else {
            "Ordinary and partitioned tables"
        };
        let notice = if truncated != 0 {
            format!(
                "{kind}: {count} choices. {truncated} schema/table groups are truncated; enter missing names exactly. A missing choice does not prove absence."
            )
        } else {
            format!(
                "{kind}: {count} choices. Enter exact names if unavailable. Choices reflect the last explicit read; typing never queries PostgreSQL."
            )
        };
        div()
            .id("pg-tool-choices")
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .p_2()
                    .children((17..23).map(|index| self.button(index, cx))),
            )
            .child(
                div()
                    .id("pg-tool-choice-limits")
                    .role(Role::Label)
                    .aria_label(notice.clone())
                    .px_2()
                    .child(notice),
            )
            .when(self.metadata.capture.is_some(), |body| {
                body.child(
                    div()
                        .id("pg-tool-choice-list")
                        .role(Role::ListBox)
                        .aria_label(format!("{kind}; arrows select, Enter uses exact name"))
                        .track_focus(&self.choice_focus)
                        .tab_index(0)
                        .h(px(140.))
                        .min_h_0()
                        .when(count == 0, |list| {
                            list.child(div().px_2().child(
                                if self.metadata.kind == choices::ChoiceKind::Table {
                                    "No captured tables match the exact schema field"
                                } else {
                                    "No schemas in this capture"
                                },
                            ))
                        })
                        .child(
                            uniform_list(
                                "pg-tool-choice-rows",
                                count,
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|index| {
                                            let Some(row) = this
                                                .metadata
                                                .capture
                                                .as_ref()
                                                .and_then(|capture| {
                                                    capture.visible.get(index).and_then(|index| {
                                                        capture.catalog.rows.get(*index)
                                                    })
                                                })
                                            else {
                                                return div().h(px(28.)).into_any_element();
                                            };
                                            let name = row.entry.name.clone();
                                            let revision = this.metadata.revision;
                                            let filter = this.metadata.filter_revision;
                                            let selected = this.metadata.selected == Some(index);
                                            let weak = cx.weak_entity();
                                            div()
                                                .id(("pg-tool-choice", index))
                                                .role(Role::ListBoxOption)
                                                .aria_label(name.clone())
                                                .aria_selected(selected)
                                                .h(px(28.))
                                                .px_2()
                                                .overflow_hidden()
                                                .when(selected, |row| {
                                                    row.bg(crate::style::select())
                                                })
                                                .child(name)
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.select_choice(
                                                            revision, filter, index, window, cx,
                                                        )
                                                    },
                                                ))
                                                .on_a11y_action(
                                                    gpui::accesskit::Action::Click,
                                                    move |_, window, cx| {
                                                        weak.update(cx, |this, cx| {
                                                            this.select_choice(
                                                                revision, filter, index, window, cx,
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
                            .track_scroll(&self.choice_scroll)
                            .h_full(),
                        ),
                )
            })
            .into_any_element()
    }
    fn select_choice(
        &mut self,
        revision: u64,
        filter: u64,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if revision != self.metadata.revision || filter != self.metadata.filter_revision {
            return;
        }
        if self
            .metadata
            .capture
            .as_ref()
            .is_none_or(|capture| index >= capture.visible.len())
        {
            return;
        }
        self.metadata.selected = Some(index);
        window.focus(&self.choice_focus, cx);
        cx.notify();
    }
    pub(super) fn choice_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.choice_focus.is_focused(window) {
            return false;
        }
        let count = self
            .metadata
            .capture
            .as_ref()
            .map_or(0, |capture| capture.visible.len());
        if count == 0 {
            return false;
        }
        let selected = self.metadata.selected;
        let next = match event.keystroke.key.as_str() {
            "up" => selected.unwrap_or(0).saturating_sub(1),
            "down" => selected.map_or(0, |index| (index + 1).min(count - 1)),
            "home" => 0,
            "end" => count - 1,
            "pageup" => selected.unwrap_or(0).saturating_sub(5),
            "pagedown" => selected.map_or(0, |index| (index + 5).min(count - 1)),
            "enter" => {
                self.activate(Action::UseChoice, window, cx);
                return true;
            }
            _ => return false,
        };
        self.metadata.selected = Some(next);
        self.choice_scroll
            .scroll_to_item(next, gpui::ScrollStrategy::Top);
        cx.notify();
        true
    }
}

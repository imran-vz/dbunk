use super::*;
/// Fixed heights of the seed pick lists (connections and jobs; columns;
/// modes and reviewed columns).
const LIST_HEIGHT: f32 = 120.;
const COLUMNS_HEIGHT: f32 = 160.;
const CHOICES_HEIGHT: f32 = 150.;
impl TableSeedView {
    pub(super) fn choices(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .connections
            .as_ref()
            .map_or(0, |choices| choices.rows.len());
        uniform_list(
            "seed-connection-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.choice_focus.is_focused(window);
                range
                    .filter_map(|index| {
                        let (id, name) = this.connections.as_ref()?.rows.get(index)?;
                        // The AX label keeps the id to tell same-named
                        // connections apart; the row shows the name only.
                        let text = format!("{name} ({id})");
                        let name = name.clone();
                        let revision = this.connection_revision;
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("seed-connection", index),
                                this.choice == Some(index),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(text)
                            .aria_selected(this.choice == Some(index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose(index, revision, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.choose(index, revision, window, cx)
                                })
                                .ok();
                            })
                            .child(crate::ui::row_label(name)),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.choices_scroll)
        .h(px(LIST_HEIGHT))
        .into_any_element()
    }
    pub(super) fn jobs_list(&self, cx: &mut Context<Self>) -> AnyElement {
        uniform_list(
            "seed-job-list",
            self.ids(cx).len(),
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.job_focus.is_focused(window);
                let ids = this.ids(cx);
                range
                    .filter_map(|index| {
                        let id = *ids.get(index)?;
                        let text = this.job_label(id, cx);
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("seed-job", index),
                                this.selected == Some(id),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(text.clone())
                            .aria_selected(this.selected == Some(id))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_job(id, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| this.select_job(id, window, cx))
                                    .ok();
                            })
                            .child(crate::ui::row_label(text)),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.jobs_scroll)
        .h(px(LIST_HEIGHT))
        .into_any_element()
    }
    pub(super) fn columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .recipe
            .as_ref()
            .map_or(0, |recipe| recipe.columns.len());
        uniform_list(
            "seed-column-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.column_focus.is_focused(window);
                range
                    .filter_map(|index| {
                        let recipe = this.recipe.as_ref()?;
                        let column = recipe.columns.get(index)?;
                        let draft = recipe.drafts.get(index)?;
                        let text = format!(
                            "{}: {}{}",
                            column.name,
                            recipe::mode_label(draft.mode),
                            if draft.null_percent.trim().is_empty() {
                                String::new()
                            } else {
                                format!(" · {}% NULL", draft.null_percent)
                            }
                        );
                        let id = recipe.setup.attempt;
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("seed-column", index),
                                this.column == Some(index),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(text.clone())
                            .aria_selected(this.column == Some(index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_column(id, index, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.select_column(id, index, window, cx)
                                })
                                .ok();
                            })
                            .child(crate::ui::row_label(text)),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.columns_scroll)
        .h(px(COLUMNS_HEIGHT))
        .into_any_element()
    }
    pub(super) fn modes(&self, cx: &mut Context<Self>) -> AnyElement {
        uniform_list(
            "seed-mode-list",
            recipe::MODE_COUNT,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.mode_focus.is_focused(window);
                range
                    .filter_map(|index| {
                        let mode = recipe::mode_at(index)?;
                        let id = this.recipe.as_ref()?.setup.attempt;
                        let column = this.column?;
                        let text = recipe::mode_label(mode);
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("seed-mode", index),
                                this.mode_choice == Some(index),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(text)
                            .aria_selected(this.mode_choice == Some(index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_mode(id, column, index, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.select_mode(id, column, index, window, cx)
                                })
                                .ok();
                            })
                            .child(crate::ui::row_label(text)),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.modes_scroll)
        .h(px(CHOICES_HEIGHT))
        .into_any_element()
    }
    pub(super) fn review_columns(&self, cx: &mut Context<Self>) -> AnyElement {
        let count = self
            .store
            .read(cx)
            .review_payload()
            .filter(|review| Some(review.attempt_id()) == self.selected)
            .map_or(0, |review| review.columns().len());
        uniform_list(
            "seed-review-column-list",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                let focused = this.review_focus.is_focused(window);
                range
                    .filter_map(|index| {
                        let review = this
                            .store
                            .read(cx)
                            .review_payload()
                            .filter(|review| Some(review.attempt_id()) == this.selected)?;
                        let column = review.columns().get(index)?;
                        let id = review.attempt_id();
                        let text = format!(
                            "{}: {}",
                            column.name,
                            details::column_action(&column.action)
                        );
                        let weak = cx.weak_entity();
                        Some(
                            crate::ui::list_row(
                                ("seed-reviewed-column", index),
                                this.review_column == Some(index),
                                focused,
                            )
                            .role(Role::ListBoxOption)
                            .aria_label(text.clone())
                            .aria_selected(this.review_column == Some(index))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.select_review_column(id, index, window, cx)
                            }))
                            .on_a11y_action(gpui::accesskit::Action::Click, move |_, window, cx| {
                                weak.update(cx, |this, cx| {
                                    this.select_review_column(id, index, window, cx)
                                })
                                .ok();
                            })
                            .child(crate::ui::row_label(text)),
                        )
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.review_scroll)
        .h(px(CHOICES_HEIGHT))
        .into_any_element()
    }
    fn select_review_column(
        &mut self,
        id: TableSeedAttemptId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        if self.selected == Some(id)
            && self
                .store
                .read(cx)
                .review_payload()
                .is_some_and(|review| review.attempt_id() == id && index < review.columns().len())
        {
            self.review_column = Some(index);
            window.focus(&self.review_focus, cx);
            cx.notify();
        }
    }
}

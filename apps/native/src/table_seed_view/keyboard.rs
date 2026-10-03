use super::*;
impl TableSeedView {
    /// Editor key bindings run before raw key capture. Route multiline Tab
    /// through the same form order without changing a recipe's literal text.
    pub(super) fn focus_control(
        &mut self,
        reverse: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composing(window, cx) {
            return;
        }
        let handles = self.focus_order(cx);
        if handles.is_empty() {
            return;
        }
        let current = handles.iter().position(|handle| handle.is_focused(window));
        let next = if reverse {
            current.map_or(handles.len() - 1, |i| {
                (i + handles.len() - 1) % handles.len()
            })
        } else {
            current.map_or(0, |i| (i + 1) % handles.len())
        };
        window.focus(&handles[next], cx);
        cx.notify();
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
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let key = event.keystroke.key.as_str();
        if key == "tab" {
            self.focus_control(modifiers.shift, window, cx);
            cx.stop_propagation();
            window.prevent_default();
            return;
        }
        if self
            .fields
            .iter()
            .any(|field| field.focus_handle(cx).contains_focused(window, cx))
        {
            return;
        }
        if self.scroll_key(key, window, cx) {
            return;
        }
        if self.choosing && self.choice_focus.is_focused(window) {
            let count = self
                .connections
                .as_ref()
                .map_or(0, |choices| choices.rows.len());
            if let Some(index) = move_index(self.choice, count, key) {
                self.choice = Some(index);
                self.choices_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
            } else if key == "enter" {
                if let Some(index) = self.choice {
                    self.choose(index, self.connection_revision, window, cx);
                }
            } else if key == "escape" {
                self.choosing = false;
                window.focus(&self.buttons[0], cx);
            } else {
                return;
            }
        } else if self.choosing_mode && self.mode_focus.is_focused(window) {
            if let Some(index) = move_index(self.mode_choice, 30, key) {
                self.mode_choice = Some(index);
                self.modes_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
            } else if key == "enter" {
                if let (Some(recipe), Some(column), Some(index)) =
                    (&self.recipe, self.column, self.mode_choice)
                {
                    self.select_mode(recipe.attempt, column, index, window, cx);
                }
            } else if key == "escape" {
                self.choosing_mode = false;
                window.focus(&self.buttons[4], cx);
            } else {
                return;
            }
        } else if self.job_focus.is_focused(window) {
            let ids = self.ids(cx);
            let current = self
                .selected
                .and_then(|id| ids.iter().position(|item| *item == id));
            if let Some(index) = move_index(current, ids.len(), key) {
                self.select_job(ids[index], window, cx);
                self.jobs_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
            } else {
                return;
            }
        } else if self.review_focus.is_focused(window) {
            let count = self
                .store
                .read(cx)
                .review_payload()
                .filter(|review| Some(review.attempt_id()) == self.selected)
                .map_or(0, |review| review.columns().len());
            if let Some(index) = move_index(self.review_column, count, key) {
                self.review_column = Some(index);
                self.review_scroll
                    .scroll_to_item(index, ScrollStrategy::Nearest);
            } else {
                return;
            }
        } else if self.column_focus.is_focused(window) {
            if let Some(recipe) = &self.recipe {
                if let Some(index) = move_index(self.column, recipe.columns.len(), key) {
                    self.select_column(recipe.attempt, index, window, cx);
                    self.columns_scroll
                        .scroll_to_item(index, ScrollStrategy::Nearest);
                } else {
                    return;
                }
            }
        } else {
            return;
        }
        cx.stop_propagation();
        window.prevent_default();
        cx.notify();
    }
    fn scroll_key(&mut self, key: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut offset = self.scroll.offset();
        let bottom = -self.scroll.max_offset().y;
        let details = self.details_focus.is_focused(window);
        offset.y = match key {
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
}
pub(super) fn move_index(current: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    match key {
        "down" => Some(current.map_or(0, |i| (i + 1).min(count - 1))),
        "up" => Some(current.map_or(count - 1, |i| i.saturating_sub(1))),
        "home" => Some(0),
        "end" => Some(count - 1),
        _ => None,
    }
}

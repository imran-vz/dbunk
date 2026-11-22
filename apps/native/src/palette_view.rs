//! Cmd-K Open Anything overlay. One flat ranked list; the top row owns
//! Return as the query changes, and capped kinds are disclosed.
use crate::{
    accessible_editor::AccessibleEditor,
    open_anything::{self, Frecency, Item, ItemKind, Ranked},
    style, ui,
};
use dbunk_lib::backend::DevelopmentEnvironment;
use editor::{Editor, EditorEvent};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    SharedString, Subscription, UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};

const QUERY_BYTES: usize = 4096;
/// Overlay width.
const WIDTH: f32 = 640.;
/// Rows shown before the result list scrolls.
const VISIBLE_ROWS: usize = 12;

/// Height of the result list: it fits its rows (one `ROW` each, plus the
/// list's top and bottom inset) up to `VISIBLE_ROWS`, then scrolls.
fn list_height(rows: usize) -> f32 {
    rows.clamp(1, VISIBLE_ROWS) as f32 * style::ROW + style::S2 * 2.
}

/// Leading icon of a result row. The kind word stays in the row's AX label.
fn kind_icon(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Command => "icons/command.svg",
        ItemKind::Tab => "icons/file.svg",
        ItemKind::Connection => "icons/database_zap.svg",
        ItemKind::Schema => "icons/folder.svg",
        ItemKind::Relation => "icons/table.svg",
        ItemKind::Object => "icons/box.svg",
    }
}

pub enum PaletteEvent {
    /// Index into the items the palette was opened with.
    Chosen(usize),
    Dismissed,
}
pub struct PaletteView<C: 'static> {
    items: Vec<Item<C>>,
    ranked: Ranked,
    selected: usize,
    note: Option<String>,
    /// Environment of each item's connection, parallel to `items`; shown as
    /// an `env_tag` at the end of the row.
    environments: Vec<Option<DevelopmentEnvironment>>,
    query: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    _query_events: Subscription,
    root: FocusHandle,
    scroll: UniformListScrollHandle,
}
impl<C: 'static> EventEmitter<PaletteEvent> for PaletteView<C> {}
impl<C: 'static> PaletteView<C> {
    /// `note` is a short line for a partial index (objects not loaded yet);
    /// `environments` is parallel to `items` (missing entries show no tag).
    pub fn new(
        items: Vec<Item<C>>,
        environments: Vec<Option<DevelopmentEnvironment>>,
        frecency: &Frecency,
        note: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = cx.new(|cx| Editor::single_line(window, cx));
        let accessible = cx.new(|cx| {
            AccessibleEditor::field(
                query.clone(),
                "Open anything: type to search, > for commands",
                false,
                cx,
            )
        });
        let ranked = open_anything::rank(&items, "", frecency);
        // Ranking uses the frecency snapshot taken when the palette opened.
        let snapshot = frecency.clone();
        let query_events = cx.subscribe_in(
            &query,
            window,
            move |this, editor, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::BufferEdited)
                    && editor
                        .update(cx, |editor, cx| editor.marked_text_range(window, cx))
                        .is_none()
                {
                    this.rerank(&snapshot, cx);
                }
            },
        );
        window.focus(&query.focus_handle(cx), cx);
        Self {
            items,
            ranked,
            selected: 0,
            note,
            environments,
            query,
            accessible,
            _query_events: query_events,
            root: cx.focus_handle(),
            scroll: UniformListScrollHandle::new(),
        }
    }
    fn rerank(&mut self, frecency: &Frecency, cx: &mut Context<Self>) {
        let buffer = self.query.read(cx).buffer().read(cx);
        if buffer.len(cx).0 > QUERY_BYTES {
            return;
        }
        let text = self.query.read(cx).text(cx);
        self.ranked = open_anything::rank(&self.items, &text, frecency);
        self.selected = 0;
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }
    /// Closes without choosing, as Escape does (a click outside the palette).
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }
    fn choose(&mut self, position: usize, cx: &mut Context<Self>) {
        if let Some(&index) = self.ranked.items.get(position) {
            cx.emit(PaletteEvent::Chosen(index));
        }
    }
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .query
            .update(cx, |editor, cx| editor.marked_text_range(window, cx))
            .is_some()
        {
            return;
        }
        let modifiers = &event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        let last = self.ranked.items.len().saturating_sub(1);
        match event.keystroke.key.as_str() {
            "escape" => cx.emit(PaletteEvent::Dismissed),
            // The query field is the dialog's only focus stop; keep focus here.
            "tab" => {}
            "enter" => self.choose(self.selected, cx),
            "down" => self.selected = (self.selected + 1).min(last),
            "up" => self.selected = self.selected.saturating_sub(1),
            "pagedown" => self.selected = (self.selected + 10).min(last),
            "pageup" => self.selected = self.selected.saturating_sub(10),
            _ => return,
        }
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        cx.notify();
        cx.stop_propagation();
    }
    fn row_label(&self, index: usize) -> String {
        let item = &self.items[index];
        if item.description.is_empty() {
            format!("{}: {}", item.kind.label(), item.label)
        } else {
            format!(
                "{}: {}, {}",
                item.kind.label(),
                item.label,
                item.description
            )
        }
    }
}
impl<C: 'static> Focusable for PaletteView<C> {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.query.focus_handle(cx)
    }
}
impl<C: 'static> Render for PaletteView<C> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self
            .ranked
            .items
            .get(self.selected)
            .map(|&index| self.row_label(index))
            .unwrap_or_else(|| "No matches".into());
        let truncated = self
            .ranked
            .truncated
            .iter()
            .map(|(kind, count)| format!("{count} more {}", kind.label().to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ");
        let count = self.ranked.items.len();
        div()
            .id("open-anything")
            .role(Role::Dialog)
            .aria_label("Open anything")
            .track_focus(&self.root)
            .capture_key_down(cx.listener(Self::key_down))
            // Clicks inside stay inside; the host's backdrop takes clicks
            // outside and calls `dismiss`.
            .occlude()
            .w(px(WIDTH))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(style::R_LG))
            .bg(style::panel())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .border_1()
            .border_color(style::line())
            .shadow_lg()
            .child(
                div()
                    .h(px(style::BAR + style::S2))
                    .px(px(style::S5))
                    .flex()
                    .items_center()
                    .gap(px(style::S4))
                    .border_b_1()
                    .border_color(style::line_soft())
                    .child(
                        gpui::svg()
                            .path("icons/magnifying_glass.svg")
                            .size(px(style::ICON))
                            .flex_none()
                            .text_color(style::dim()),
                    )
                    .child(div().flex_1().min_w_0().child(self.accessible.clone())),
            )
            .child(
                div()
                    .id("open-anything-results")
                    .role(Role::ListBox)
                    .aria_label(format!(
                        "{count} results; arrows select, Return opens, Escape closes"
                    ))
                    .aria_value(selected)
                    .flex_none()
                    .py(px(style::S2))
                    .map(|results| {
                        if count == 0 {
                            return results.child(
                                div()
                                    .h(px(style::ROW))
                                    .px(px(style::S5))
                                    .flex()
                                    .items_center()
                                    .text_color(style::dim())
                                    .child("No matches"),
                            );
                        }
                        results.child(
                            uniform_list(
                                "open-anything-rows",
                                count,
                                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                    range
                                        .map(|position| {
                                            let index = this.ranked.items[position];
                                            let item = &this.items[index];
                                            let selected = position == this.selected;
                                            let environment =
                                                this.environments.get(index).copied().flatten();
                                            let row = ui::list_row(
                                                ("open-anything-row", position),
                                                selected,
                                                true,
                                            )
                                            .role(Role::ListBoxOption)
                                            .aria_label(this.row_label(index))
                                            .aria_selected(selected)
                                            .gap(px(style::S4))
                                            .child(
                                                gpui::svg()
                                                    .path(kind_icon(item.kind))
                                                    .size(px(style::ICON))
                                                    .flex_none()
                                                    .text_color(if selected {
                                                        style::text()
                                                    } else {
                                                        style::dim()
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .flex_none()
                                                    .max_w(px(WIDTH * 0.5))
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_color(style::text())
                                                    .child(SharedString::from(item.label.clone())),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .text_size(px(style::FONT_SMALL))
                                                    .text_color(style::dim())
                                                    .child(SharedString::from(
                                                        item.description.clone(),
                                                    )),
                                            )
                                            .when_some(environment, |row, environment| {
                                                row.child(ui::env_tag(environment))
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.selected = position;
                                                this.choose(position, cx);
                                            }));
                                            // A full-width flex column, so the
                                            // highlight spans the panel.
                                            div().w_full().flex().flex_col().child(row)
                                        })
                                        .collect()
                                }),
                            )
                            .track_scroll(&self.scroll)
                            .h(px(list_height(count) - style::S2 * 2.)),
                        )
                    }),
            )
            .child(
                div()
                    .h(px(style::FOOTER))
                    .flex_none()
                    .px(px(style::S5))
                    .flex()
                    .items_center()
                    .gap(px(style::S5))
                    .border_t_1()
                    .border_color(style::line_soft())
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::dim())
                    .whitespace_nowrap()
                    .map(|footer| {
                        // A partial result says so; otherwise the footer
                        // only carries the keys.
                        if !truncated.is_empty() {
                            footer.child(
                                div()
                                    .id("open-anything-truncated")
                                    .role(Role::Label)
                                    .aria_label(format!("Not shown: {truncated}"))
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .text_color(style::warn())
                                    .child(format!("{truncated} not shown; refine the search")),
                            )
                        } else if let Some(note) = self.note.clone() {
                            footer.child(
                                div()
                                    .id("open-anything-note")
                                    .role(Role::Label)
                                    .aria_label(note.clone())
                                    .min_w_0()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(note),
                            )
                        } else {
                            footer
                        }
                    })
                    .child(div().flex_1())
                    .child(div().flex_none().child("↑↓ select   ↵ open   esc close")),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_list_fits_its_rows_up_to_a_cap() {
        let inset = style::S2 * 2.;
        // An empty list keeps one row for "No matches".
        assert_eq!(list_height(0), style::ROW + inset);
        assert_eq!(list_height(3), 3. * style::ROW + inset);
        assert_eq!(list_height(VISIBLE_ROWS), list_height(500));
        assert!(list_height(VISIBLE_ROWS) < 400.);
    }

    #[test]
    fn every_kind_has_its_own_icon() {
        let kinds = [
            ItemKind::Command,
            ItemKind::Tab,
            ItemKind::Connection,
            ItemKind::Schema,
            ItemKind::Relation,
            ItemKind::Object,
        ];
        for (i, a) in kinds.iter().enumerate() {
            assert!(
                kinds[i + 1..]
                    .iter()
                    .all(|b| kind_icon(*a) != kind_icon(*b))
            );
        }
    }
}

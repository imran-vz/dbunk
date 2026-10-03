//! Cmd-K Open Anything overlay. One flat ranked list; the top row owns
//! Return as the query changes, and capped kinds are disclosed.
use crate::{
    accessible_editor::AccessibleEditor,
    open_anything::{self, Frecency, Item, Ranked},
};
use editor::{Editor, EditorEvent};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    SharedString, Subscription, UniformListScrollHandle, Window, div, prelude::*, px, rgb,
    uniform_list,
};

const QUERY_BYTES: usize = 4096;

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
    query: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    _query_events: Subscription,
    root: FocusHandle,
    scroll: UniformListScrollHandle,
}
impl<C: 'static> EventEmitter<PaletteEvent> for PaletteView<C> {}
impl<C: 'static> PaletteView<C> {
    /// `note` discloses sources the index deliberately does not cover.
    pub fn new(
        items: Vec<Item<C>>,
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
        div()
            .id("open-anything")
            .role(Role::Dialog)
            .aria_label("Open anything")
            .track_focus(&self.root)
            .capture_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismissed)))
            .w(px(640.))
            .max_h(px(480.))
            .flex()
            .flex_col()
            .bg(rgb(0))
            .text_color(rgb(0xffffff))
            .border_1()
            .border_color(rgb(0x666666))
            .child(div().h(px(28.)).px_2().child(self.accessible.clone()))
            .child(
                div()
                    .id("open-anything-results")
                    .role(Role::ListBox)
                    .aria_label(format!(
                        "{} results; arrows select, Return opens, Escape closes",
                        self.ranked.items.len()
                    ))
                    .aria_value(selected)
                    .flex_1()
                    .min_h(px(120.))
                    .child(
                        uniform_list(
                            "open-anything-rows",
                            self.ranked.items.len(),
                            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                                range
                                    .map(|position| {
                                        let index = this.ranked.items[position];
                                        let item = &this.items[index];
                                        let text = format!(
                                            "{}  {}  {}",
                                            item.kind.label(),
                                            item.label,
                                            item.description
                                        );
                                        let selected = position == this.selected;
                                        div()
                                            .id(("open-anything-row", position))
                                            .role(Role::ListBoxOption)
                                            .aria_label(this.row_label(index))
                                            .aria_selected(selected)
                                            .h(px(24.))
                                            .px_2()
                                            .text_sm()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .when(selected, |row| row.bg(rgb(0x252525)))
                                            .child(SharedString::from(text))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.selected = position;
                                                this.choose(position, cx);
                                            }))
                                    })
                                    .collect()
                            }),
                        )
                        .track_scroll(&self.scroll)
                        .h(px(400.)),
                    ),
            )
            .when(!truncated.is_empty(), |root| {
                root.child(
                    div()
                        .id("open-anything-truncated")
                        .role(Role::Label)
                        .aria_label(format!("Not shown: {truncated}"))
                        .px_2()
                        .text_xs()
                        .text_color(rgb(0xfbbf24))
                        .child(format!("Not shown: {truncated}; refine the search")),
                )
            })
            .when_some(self.note.clone(), |root, note| {
                root.child(
                    div()
                        .id("open-anything-note")
                        .role(Role::Label)
                        .aria_label(note.clone())
                        .px_2()
                        .text_xs()
                        .text_color(rgb(0xbbbbbb))
                        .child(note),
                )
            })
    }
}

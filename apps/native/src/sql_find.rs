//! SQL editor find bar. Zed's buffer search needs a Zed workspace pane, which
//! this host does not have, so Cmd-F opened nothing. Matching is literal and
//! case-insensitive by Unicode scalar, wraps, and only moves the selection: it
//! never edits the draft and never executes SQL.
use crate::accessible_editor::AccessibleEditor;
use editor::{Editor, EditorEvent, SelectionEffects};
use gpui::{
    Context, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    Subscription, Window, div, prelude::*, px,
};
use multi_buffer::MultiBufferOffset;
use std::ops::Range;

const QUERY_CHARS: usize = 1024;
/// Match counting stops here; the label then says "or more".
const COUNT_LIMIT: usize = 10_000;

fn fold(c: char) -> char {
    // Single-scalar folding keeps byte ranges exact in the original text.
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}
fn matches_at(text: &str, at: usize, query: &[char]) -> Option<usize> {
    let mut chars = text[at..].char_indices();
    for wanted in query {
        let (_, c) = chars.next()?;
        if fold(c) != *wanted {
            return None;
        }
    }
    Some(chars.next().map_or(text.len(), |(offset, _)| at + offset))
}

/// Byte ranges of every non-overlapping match, capped at `COUNT_LIMIT`.
pub fn matches(text: &str, query: &str) -> Vec<Range<usize>> {
    let query: Vec<char> = query.chars().map(fold).collect();
    if query.is_empty() {
        return Vec::new();
    }
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() && found.len() < COUNT_LIMIT {
        if let Some(end) = matches_at(text, at, &query) {
            found.push(at..end);
            at = end;
        } else {
            at += text[at..].chars().next().map_or(1, char::len_utf8);
        }
    }
    found
}

/// Next match strictly after `from` (or before it when searching backwards),
/// wrapping around the draft.
pub fn next(found: &[Range<usize>], from: usize, forward: bool) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    Some(if forward {
        found
            .iter()
            .position(|range| range.start > from)
            .unwrap_or(0)
    } else {
        found
            .iter()
            .rposition(|range| range.start < from)
            .unwrap_or(found.len() - 1)
    })
}

pub enum FindEvent {
    Close,
}
pub struct FindView {
    target: Entity<Editor>,
    query: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
    _events: Subscription,
    root: FocusHandle,
    status: String,
}
impl EventEmitter<FindEvent> for FindView {}
impl FindView {
    pub fn new(target: Entity<Editor>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| Editor::single_line(window, cx));
        let accessible =
            cx.new(|cx| AccessibleEditor::field(query.clone(), "Find in SQL", false, cx));
        let events = cx.subscribe_in(
            &query,
            window,
            |this, editor, event: &EditorEvent, window, cx| {
                // Incremental search waits for committed composition.
                if matches!(event, EditorEvent::BufferEdited)
                    && editor
                        .update(cx, |editor, cx| editor.marked_text_range(window, cx))
                        .is_none()
                {
                    this.find(true, true, window, cx);
                }
            },
        );
        Self {
            target,
            query,
            accessible,
            _events: events,
            root: cx.focus_handle(),
            status: "Type to find; Return next, Shift-Return previous, Escape closes".into(),
        }
    }
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.query.focus_handle(cx), cx);
    }
    /// `incremental` re-searches from the current selection start so typing
    /// extends the same match instead of skipping past it.
    pub fn find(
        &mut self,
        forward: bool,
        incremental: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.query.read(cx).text(cx);
        if query.chars().count() > QUERY_CHARS {
            self.status = "Search text exceeds 1,024 characters".into();
            cx.notify();
            return;
        }
        let text = self.target.read(cx).text(cx);
        let found = matches(&text, &query);
        let selection = self.target.update(cx, |editor, cx| {
            editor
                .selections
                .newest::<MultiBufferOffset>(&editor.display_snapshot(cx))
                .range()
        });
        let from = if incremental {
            selection.start.0.saturating_sub(1)
        } else if forward {
            selection.start.0
        } else {
            selection.end.0.max(selection.start.0)
        };
        let from = if incremental && selection.start.0 == 0 {
            // Allow a match at offset 0 while typing.
            if found.first().is_some_and(|range| range.start == 0) {
                self.select(&found, 0, window, cx);
                return;
            }
            0
        } else {
            from
        };
        match next(&found, from, forward) {
            Some(index) => self.select(&found, index, window, cx),
            None => {
                self.status = if query.is_empty() {
                    "Type to find".into()
                } else {
                    "No matches".into()
                };
                cx.notify();
            }
        }
    }
    fn select(
        &mut self,
        found: &[Range<usize>],
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = found[index].clone();
        self.target.update(cx, |editor, cx| {
            editor.change_selections(SelectionEffects::default(), window, cx, |selections| {
                selections
                    .select_ranges([MultiBufferOffset(range.start)..MultiBufferOffset(range.end)])
            });
        });
        self.status = format!(
            "Match {} of {}{}",
            index + 1,
            found.len(),
            if found.len() >= COUNT_LIMIT {
                " or more"
            } else {
                ""
            }
        );
        cx.notify();
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
        match event.keystroke.key.as_str() {
            "enter" => self.find(!modifiers.shift, false, window, cx),
            "escape" => cx.emit(FindEvent::Close),
            _ => return,
        }
        cx.stop_propagation();
    }
}
impl Render for FindView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sql-find")
            .role(Role::Search)
            .aria_label("Find in SQL")
            .track_focus(&self.root)
            .capture_key_down(cx.listener(Self::key_down))
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(crate::style::line())
            .child(div().w(px(280.)).h(px(24.)).child(self.accessible.clone()))
            .child(
                div()
                    .id("sql-find-status")
                    .role(Role::Status)
                    .aria_label(self.status.clone())
                    .text_xs()
                    .text_color(crate::style::dim())
                    .child(self.status.clone()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_case_insensitive_matches_keep_exact_byte_ranges() {
        let text = "SELECT ÉTÉ, été FROM t WHERE été = 'Été'";
        let found = matches(text, "été");
        assert_eq!(found.len(), 4);
        for range in &found {
            assert_eq!(text[range.clone()].to_lowercase(), "été");
        }
        // Regex-looking text is literal.
        assert_eq!(matches("a.b axb", "a.b"), vec![0..3]);
        // Non-overlapping, and an empty query matches nothing.
        assert_eq!(matches("aaaa", "aa"), vec![0..2, 2..4]);
        assert!(matches("abc", "").is_empty());
        // A multi-scalar lowercase (İ) folds to itself rather than shifting bytes.
        assert_eq!(matches("İstanbul", "İst"), vec![0..4]);
    }

    #[test]
    fn next_and_previous_wrap_around_the_draft() {
        let found = vec![2..4, 10..12, 20..22];
        assert_eq!(next(&found, 0, true), Some(0));
        assert_eq!(next(&found, 2, true), Some(1));
        assert_eq!(next(&found, 20, true), Some(0));
        assert_eq!(next(&found, 10, false), Some(0));
        assert_eq!(next(&found, 2, false), Some(2));
        assert_eq!(next(&[], 0, true), None);
    }

    #[test]
    fn counting_is_bounded() {
        let text = "x".repeat(COUNT_LIMIT + 50);
        assert_eq!(matches(&text, "x").len(), COUNT_LIMIT);
    }
}

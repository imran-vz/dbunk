//! Text semantics for the pinned Zed editor using GPUI's public AccessKit hook.
//! The editor remains the only editing model; this view publishes its primary
//! selection and translates assistive selection requests back into that model.

use std::{cell::RefCell, rc::Rc};

use editor::display_map::{DisplayPoint, DisplayRow, ToDisplayPoint};
use editor::{Editor, EditorEvent, SelectionEffects};
use gpui::{
    Bounds, Context, Entity, Focusable, Pixels, SharedString, Subscription, Window,
    accesskit::{
        Action, ActionData, Node, NodeId, Rect, Role, TextDirection, TextPosition, TextSelection,
    },
    canvas, div,
    prelude::*,
};
use multi_buffer::MultiBufferOffset;
use text::Bias;
use unicode_segmentation::UnicodeSegmentation;

pub struct AccessibleEditor {
    editor: Entity<Editor>,
    label: SharedString,
    document: Option<Rc<TextDocument>>,
    revision: u64,
    single_line: bool,
    secret: bool,
    _subscription: Subscription,
}

impl AccessibleEditor {
    pub fn new(
        editor: Entity<Editor>,
        label: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscription = cx.subscribe(&editor, |this, _, event, cx| match event {
            EditorEvent::BufferEdited => {
                this.document = None;
                this.revision += 1;
                cx.notify();
            }
            EditorEvent::SelectionsChanged { .. }
            | EditorEvent::ScrollPositionChanged { .. }
            | EditorEvent::Focused
            | EditorEvent::Blurred => {
                cx.notify();
            }
            _ => {}
        });
        Self {
            editor,
            label: label.into(),
            document: None,
            revision: 0,
            single_line: false,
            secret: false,
            _subscription: subscription,
        }
    }

    pub fn field(
        editor: Entity<Editor>,
        label: impl Into<SharedString>,
        secret: bool,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut field = Self::new(editor, label, cx);
        field.single_line = true;
        field.secret = secret;
        field
    }
}

impl Render for AccessibleEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut element = div()
            .id("accessible-editor")
            .role(if self.secret {
                Role::PasswordInput
            } else if self.single_line {
                Role::TextInput
            } else {
                Role::MultilineTextInput
            })
            .aria_label(self.label.clone())
            .track_focus(&self.editor.focus_handle(cx))
            .relative()
            .size_full()
            .child(self.editor.clone());

        if self.secret {
            // Secure fields never publish the real buffer to the accessibility
            // tree or clipboard. The editor still owns IME and native selection.
            let masked = "•".repeat(self.editor.read(cx).text(cx).chars().count());
            return element
                .capture_action(|_: &editor::actions::Copy, _, cx| cx.stop_propagation())
                .capture_action(|_: &editor::actions::Cut, _, cx| cx.stop_propagation())
                .capture_action(|_: &editor::actions::CopyAndTrim, _, cx| cx.stop_propagation())
                .capture_action(|_: &editor::actions::CopyHighlightJson, _, cx| {
                    cx.stop_propagation()
                })
                .capture_action(|_: &editor::actions::CutToEndOfLine, _, cx| cx.stop_propagation())
                .capture_action(|_: &editor::actions::KillRingCut, _, cx| cx.stop_propagation())
                .a11y_synthetic_children(move |builder| {
                    builder.parent_node().set_value(masked.clone())
                });
        }

        // Large documents incur no text-tree work until an assistive client
        // activates accessibility. Cache text runs across caret-only changes.
        if window.is_a11y_active() {
            let document = self
                .document
                .get_or_insert_with(|| Rc::new(TextDocument::new(self.editor.read(cx).text(cx))))
                .clone();
            let published = Rc::new(RefCell::new(document.clone()));
            let layout_document = published.clone();
            let action_document = published.clone();
            let geometry = Rc::new(RefCell::new(Vec::new()));
            let layout_geometry = geometry.clone();
            let selection = Rc::new(RefCell::new((0, 0)));
            let layout_selection = selection.clone();
            let ids = Rc::new(RefCell::new(Vec::new()));
            let action_ids = ids.clone();
            let editor = self.editor.clone();
            let layout_editor = editor.clone();
            let revision = self.revision;

            element = element
                // This runs after the real editor's prepaint, so wrapping,
                // autoscroll and resized bounds belong to the current frame.
                .child(
                    canvas(
                        move |bounds, window, cx| {
                            layout_editor.update(cx, |editor, cx| {
                                let (document, geometry) =
                                    document.layout(editor, bounds, window, cx);
                                let current = editor
                                    .selections
                                    .newest::<MultiBufferOffset>(&editor.display_snapshot(cx));
                                *layout_selection.borrow_mut() =
                                    (current.tail().0, current.head().0);
                                *layout_document.borrow_mut() = Rc::new(document);
                                *layout_geometry.borrow_mut() = geometry;
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .a11y_synthetic_children(move |builder| {
                    let document = published.borrow();
                    let geometry = geometry.borrow();
                    let mut ids = ids.borrow_mut();
                    // AccessKit derives a value from runs for single-line
                    // inputs only; multiline inputs need an explicit value.
                    builder.parent_node().set_value(document.text.as_ref());
                    for (index, run) in document.runs.iter().enumerate() {
                        // Keep ids stable across caret moves, but reject an
                        // action carrying text positions from an older edit.
                        let id = builder.synthetic_node_id((revision, run.start, run.end));
                        ids.push(id);
                        let mut node = Node::new(Role::TextRun);
                        node.set_value(&document.text[run.start..run.end]);
                        node.set_character_lengths(run.lengths.clone());
                        if let Some(geometry) = &geometry[index] {
                            node.set_bounds(geometry.bounds);
                            node.set_text_direction(TextDirection::LeftToRight);
                            node.set_character_positions(geometry.positions.clone());
                            node.set_character_widths(geometry.widths.clone());
                        }
                        builder.push_child(id, node);
                    }
                    builder.parent_node().set_text_selection(TextSelection {
                        anchor: document.position(selection.borrow().0, &ids),
                        focus: document.position(selection.borrow().1, &ids),
                    });
                })
                .on_a11y_action(Action::SetTextSelection, move |data, window, cx| {
                    let Some(ActionData::SetTextSelection(selection)) = data else {
                        return;
                    };
                    let ids = action_ids.borrow();
                    let action_document = action_document.borrow();
                    let Some(anchor) = action_document.offset(selection.anchor, &ids) else {
                        return;
                    };
                    let Some(focus) = action_document.offset(selection.focus, &ids) else {
                        return;
                    };
                    editor.update(cx, |editor, cx| {
                        // The buffer may have changed since the published frame.
                        // Never apply offsets from a retired text snapshot.
                        if editor.text(cx) != action_document.text.as_ref() {
                            return;
                        }
                        window.focus(&editor.focus_handle(cx), cx);
                        editor.change_selections(
                            SelectionEffects::default().completions(false),
                            window,
                            cx,
                            |selections| {
                                selections.select_ranges([
                                    MultiBufferOffset(anchor)..MultiBufferOffset(focus)
                                ])
                            },
                        );
                    });
                });
        }
        element
    }
}

#[derive(Clone)]
struct TextRun {
    start: usize,
    end: usize,
    lengths: Vec<u8>,
}

struct TextDocument {
    text: Rc<str>,
    runs: Vec<TextRun>,
}

struct RunGeometry {
    bounds: Rect,
    positions: Vec<f32>,
    widths: Vec<f32>,
}

impl TextDocument {
    fn new(text: String) -> Self {
        let mut runs = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let end = start + line.len();
            // Match Zed's character movement, including combining marks,
            // emoji sequences and CRLF. AccessKit stores each length in u8;
            // unusually long clusters must be split without truncating bytes.
            let mut lengths = Vec::new();
            for grapheme in line.graphemes(true) {
                if let Ok(length) = u8::try_from(grapheme.len()) {
                    lengths.push(length);
                } else {
                    lengths.extend(grapheme.chars().map(|ch| ch.len_utf8() as u8));
                }
            }
            runs.push(TextRun {
                start,
                end,
                lengths,
            });
            start = end;
        }
        // An empty final run represents an empty document or a caret on the
        // blank line after a trailing newline, not the preceding line break.
        if text.is_empty() || text.ends_with('\n') {
            runs.push(TextRun {
                start,
                end: start,
                lengths: Vec::new(),
            });
        }
        Self {
            text: text.into(),
            runs,
        }
    }

    /// Split cached logical runs only where visible display rows require it.
    /// Offscreen text stays readable without shaping the whole document.
    fn split_runs(&self, breaks: &[usize]) -> Self {
        let mut runs = Vec::new();
        for run in &self.runs {
            let first_break = breaks.partition_point(|offset| *offset <= run.start);
            if breaks
                .get(first_break)
                .is_none_or(|offset| *offset >= run.end)
            {
                runs.push(run.clone());
                continue;
            }
            let mut start = run.start;
            let mut first = 0;
            let mut offset = run.start;
            for (index, length) in run.lengths.iter().enumerate() {
                if offset > start && breaks.binary_search(&offset).is_ok() {
                    runs.push(TextRun {
                        start,
                        end: offset,
                        lengths: run.lengths[first..index].to_vec(),
                    });
                    start = offset;
                    first = index;
                }
                offset += usize::from(*length);
            }
            runs.push(TextRun {
                start,
                end: run.end,
                lengths: run.lengths[first..].to_vec(),
            });
        }
        Self {
            text: self.text.clone(),
            runs,
        }
    }

    fn layout(
        &self,
        editor: &mut Editor,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) -> (Self, Vec<Option<RunGeometry>>) {
        let snapshot = editor.snapshot(window, cx);
        let details = editor.text_layout_details(window, cx);
        let style = editor.style(cx).clone();
        let font = window.text_system().resolve_font(&style.text.font());
        let font_size = style.text.font_size.to_pixels(window.rem_size());
        let line_height = style.text.line_height_in_pixels(window.rem_size());
        let em = window.text_system().em_layout_width(font, font_size);
        let gutter = snapshot.gutter_dimensions(font, font_size, &style, window, cx);
        let scroll = snapshot.scroll_position();
        let scroll_x = window.pixel_snap_f64(scroll.x * f64::from(em));
        let scroll_y = window.pixel_snap_f64(scroll.y * f64::from(line_height));
        let first = (scroll_y / f64::from(line_height)).floor().max(0.) as u32;
        let last = ((scroll_y + f64::from(bounds.size.height)) / f64::from(line_height))
            .ceil()
            .max(0.) as u32;
        let last = last.min(snapshot.max_point().row().0);
        let mut breaks = Vec::new();
        for row in first..=last.saturating_add(1).min(snapshot.max_point().row().0) {
            breaks.push(
                DisplayPoint::new(DisplayRow(row), 0)
                    .to_offset(&snapshot, Bias::Left)
                    .0,
            );
        }
        breaks.sort_unstable();
        breaks.dedup();
        let document = self.split_runs(&breaks);
        let scale = f64::from(window.scale_factor());
        let mut shaped = std::collections::HashMap::new();
        let geometry = document
            .runs
            .iter()
            .map(|run| {
                let start = MultiBufferOffset(run.start).to_display_point(&snapshot);
                let row = start.row();
                if row.0 < first || row.0 > last {
                    return None;
                }
                let line = shaped
                    .entry(row.0)
                    .or_insert_with(|| snapshot.layout_row(row, &details));
                let left = f64::from(line.x_for_index(start.column() as usize));
                let mut offset = run.start;
                let mut edges = vec![left];
                for length in &run.lengths {
                    offset += usize::from(*length);
                    let point = MultiBufferOffset(offset).to_display_point(&snapshot);
                    let x = if point.row() == row {
                        line.x_for_index(point.column() as usize)
                    } else {
                        line.width
                    };
                    edges.push(f64::from(x));
                }
                let x =
                    (f64::from(bounds.origin.x + gutter.full_width()) + left - scroll_x) * scale;
                let y = (f64::from(bounds.origin.y) + f64::from(line_height) * f64::from(row.0)
                    - scroll_y)
                    * scale;
                Some(RunGeometry {
                    bounds: Rect::new(
                        x,
                        y,
                        x + (edges.last().copied().unwrap_or(left) - left).max(0.) * scale,
                        y + f64::from(line_height) * scale,
                    ),
                    positions: edges[..run.lengths.len()]
                        .iter()
                        .map(|x| ((x - left) * scale) as f32)
                        .collect(),
                    widths: edges
                        .windows(2)
                        .map(|pair| ((pair[1] - pair[0]).max(0.) * scale) as f32)
                        .collect(),
                })
            })
            .collect();
        (document, geometry)
    }

    fn position(&self, offset: usize, ids: &[NodeId]) -> TextPosition {
        let offset = offset.min(self.text.len());
        let index = self
            .runs
            .partition_point(|run| run.start <= offset)
            .saturating_sub(1);
        let run = &self.runs[index];
        let mut bytes = run.start;
        let character_index = run
            .lengths
            .iter()
            .take_while(|length| {
                bytes += usize::from(**length);
                bytes <= offset
            })
            .count();
        TextPosition {
            node: ids[index],
            character_index,
        }
    }

    fn offset(&self, position: TextPosition, ids: &[NodeId]) -> Option<usize> {
        let index = ids.iter().position(|id| *id == position.node)?;
        let run = self.runs.get(index)?;
        if position.character_index > run.lengths.len() {
            return None;
        }
        Some(
            run.start
                + run.lengths[..position.character_index]
                    .iter()
                    .map(|n| usize::from(*n))
                    .sum::<usize>(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_splits_preserve_unicode_offsets_and_ignore_interior_grapheme_breaks() {
        let document = TextDocument::new("ab😀e\u{301}cd\n\n".into());
        let split = document.split_runs(&[2, 4, 6, 7, 9]);
        assert_eq!(
            split
                .runs
                .iter()
                .map(|run| (run.start, run.end))
                .collect::<Vec<_>>(),
            [(0, 2), (2, 6), (6, 9), (9, 12), (12, 13), (13, 13)]
        );
        let ids = (1..=split.runs.len())
            .map(|id| NodeId(id as u64))
            .collect::<Vec<_>>();
        for offset in document
            .text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain([document.text.len()])
        {
            assert_eq!(
                split.offset(split.position(offset, &ids), &ids),
                Some(offset)
            );
        }
    }

    #[test]
    fn unicode_positions_round_trip_at_every_selectable_boundary() {
        for text in ["", "SELECT 1;", "é😀e\u{301}\nSELECT 2;\n", "a\r\nb"] {
            let document = TextDocument::new(text.into());
            let ids = (1..=document.runs.len())
                .map(|id| NodeId(id as u64))
                .collect::<Vec<_>>();
            for offset in text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .chain([text.len()])
            {
                assert_eq!(
                    document.offset(document.position(offset, &ids), &ids),
                    Some(offset)
                );
            }
            for run in &document.runs {
                assert_eq!(
                    run.lengths.iter().map(|n| usize::from(*n)).sum::<usize>(),
                    run.end - run.start
                );
            }
            let end = document.position(text.len(), &ids);
            assert_eq!(end.node, *ids.last().unwrap());
        }
    }

    #[test]
    fn rejects_unknown_nodes_and_out_of_range_positions() {
        let document = TextDocument::new("é".into());
        let ids = [NodeId(7)];
        assert_eq!(
            document.offset(
                TextPosition {
                    node: NodeId(8),
                    character_index: 0
                },
                &ids
            ),
            None
        );
        assert_eq!(
            document.offset(
                TextPosition {
                    node: NodeId(7),
                    character_index: 2
                },
                &ids
            ),
            None
        );
    }

    #[test]
    fn long_combining_clusters_preserve_every_byte() {
        let text = format!("e{}", "\u{301}".repeat(300));
        let document = TextDocument::new(text.clone());
        assert_eq!(
            document.runs[0]
                .lengths
                .iter()
                .map(|n| usize::from(*n))
                .sum::<usize>(),
            text.len()
        );
        let ids = [NodeId(1)];
        assert_eq!(
            document.offset(document.position(text.len(), &ids), &ids),
            Some(text.len())
        );
    }
}

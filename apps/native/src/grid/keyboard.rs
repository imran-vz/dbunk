//! Retained-grid navigation never fetches another page or changes source identity.
use super::*;

#[derive(Clone, Copy)]
enum Jump {
    RowStart,
    RowEnd,
    First,
    Last,
    PageUp,
    PageDown,
}

fn destination(
    current: Option<(usize, usize)>,
    rows: usize,
    columns: usize,
    page_rows: usize,
    jump: Jump,
) -> Option<(usize, usize)> {
    if rows == 0 || columns == 0 {
        return None;
    }
    let (row, column) = current.unwrap_or((0, 0));
    let (row, column) = (row.min(rows - 1), column.min(columns - 1));
    let page = page_rows.max(1);
    Some(match jump {
        Jump::RowStart => (row, 0),
        Jump::RowEnd => (row, columns - 1),
        Jump::First => (0, 0),
        Jump::Last => (rows - 1, columns - 1),
        Jump::PageUp => (row.saturating_sub(page), column),
        Jump::PageDown => (row.saturating_add(page).min(rows - 1), column),
    })
}

/// Keys that open the cell editor in table mode. `space` stays Inspect (its
/// binding runs first) and shortcuts with cmd, ctrl or alt never edit.
pub(super) fn edit_seed(key: &gpui::Keystroke) -> Option<crate::data_model::EditSeed> {
    use crate::data_model::EditSeed;
    let modifiers = &key.modifiers;
    if modifiers.platform || modifiers.control || modifiers.alt || modifiers.function {
        return None;
    }
    match key.key.as_str() {
        "enter" if !modifiers.shift => Some(EditSeed::Keep),
        "f2" => Some(EditSeed::Keep),
        "backspace" | "delete" => Some(EditSeed::Clear),
        "space" | "enter" | "tab" | "escape" => None,
        // Named keys (arrows, home, f5, …) never type text.
        name if name.chars().count() != 1 => None,
        _ => key
            .key_char
            .as_deref()
            .filter(|text| !text.is_empty() && !text.chars().any(char::is_control))
            .map(|text| EditSeed::Replace(text.to_owned())),
    }
}

impl ResultGrid {
    pub(super) fn jump_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Embedded editors and marked-text composition keep their own keys.
        if !self.focus.is_focused(window) {
            return;
        }
        let key = &event.keystroke;
        if self.table_mode()
            && let Some(seed) = edit_seed(key)
        {
            if let Some((row, source)) = self.selected_cell() {
                cx.emit(GridEvent::EditCell {
                    cell: crate::data_model::CellRef::Page(row),
                    source,
                    seed,
                });
                cx.stop_propagation();
            }
            return;
        }
        if key.modifiers.alt {
            return;
        }
        let modified = key.modifiers.platform || key.modifiers.control;
        if key.key == "escape" && !modified && !key.modifiers.shift {
            if let Some(view) = self.views.get_mut(self.model.active) {
                view.anchor = None;
                self.copy_status = None;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        let jump = match (key.key.as_str(), modified) {
            ("home", false) => Jump::RowStart,
            ("end", false) => Jump::RowEnd,
            ("home", true) => Jump::First,
            ("end", true) => Jump::Last,
            ("pageup", false) => Jump::PageUp,
            ("pagedown", false) => Jump::PageDown,
            _ => return,
        };
        let current = self.views.get(self.model.active).and_then(|view| view.head);
        let height = self.scroll().0.borrow().base_handle.bounds().size.height;
        let page_rows = ((height / ROW_HEIGHT).floor() as usize).saturating_sub(1);
        if let Some((row, column)) = destination(
            current,
            self.row_count(),
            self.column_count(),
            page_rows,
            jump,
        ) {
            if key.modifiers.shift
                && let Some(view) = self.views.get_mut(self.model.active)
                && view.anchor.is_none()
            {
                view.anchor = Some(current.unwrap_or((0, 0)));
            }
            self.select(row, column, key.modifiers.shift, cx);
            self.copy_status = None;
            self.scroll().scroll_to_item(row, ScrollStrategy::Nearest);
            self.reveal_selected_column();
        }
        cx.stop_propagation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::EditSeed;
    use gpui::{Keystroke, Modifiers};

    fn stroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: key_char.map(str::to_owned),
        }
    }

    #[test]
    fn edit_seed_maps_open_keys_and_ignores_shortcuts() {
        let plain = Modifiers::default();
        assert_eq!(
            edit_seed(&stroke("enter", Some("\n"), plain)),
            Some(EditSeed::Keep)
        );
        assert_eq!(edit_seed(&stroke("f2", None, plain)), Some(EditSeed::Keep));
        assert_eq!(
            edit_seed(&stroke("backspace", None, plain)),
            Some(EditSeed::Clear)
        );
        assert_eq!(
            edit_seed(&stroke("delete", None, plain)),
            Some(EditSeed::Clear)
        );
        assert_eq!(
            edit_seed(&stroke("a", Some("a"), plain)),
            Some(EditSeed::Replace("a".into()))
        );
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        assert_eq!(
            edit_seed(&stroke("a", Some("A"), shift)),
            Some(EditSeed::Replace("A".into()))
        );
        assert_eq!(edit_seed(&stroke("enter", Some("\n"), shift)), None);
        for (key, modifiers) in [
            (
                "a",
                Modifiers {
                    platform: true,
                    ..Modifiers::default()
                },
            ),
            (
                "x",
                Modifiers {
                    control: true,
                    ..Modifiers::default()
                },
            ),
            (
                "a",
                Modifiers {
                    alt: true,
                    ..Modifiers::default()
                },
            ),
        ] {
            assert_eq!(edit_seed(&stroke(key, Some(key), modifiers)), None, "{key}");
        }
        assert_eq!(edit_seed(&stroke("space", Some(" "), plain)), None);
        assert_eq!(edit_seed(&stroke("tab", Some("\t"), plain)), None);
        assert_eq!(edit_seed(&stroke("escape", None, plain)), None);
        assert_eq!(edit_seed(&stroke("up", None, plain)), None);
    }

    #[test]
    fn retained_page_jumps_clamp_without_changing_display_column() {
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 10, Jump::PageDown),
            Some((14, 6))
        );
        assert_eq!(
            destination(Some((14, 6)), 20, 8, 10, Jump::PageDown),
            Some((19, 6))
        );
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 10, Jump::PageUp),
            Some((0, 6))
        );
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 0, Jump::PageDown),
            Some((5, 6))
        );
        assert_eq!(
            destination(Some((4, 6)), 20, 8, usize::MAX, Jump::PageDown),
            Some((19, 6))
        );
    }
    #[test]
    fn endpoints_use_visible_dimensions_and_refuse_empty_results() {
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 1, Jump::RowStart),
            Some((4, 0))
        );
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 1, Jump::RowEnd),
            Some((4, 7))
        );
        assert_eq!(
            destination(Some((4, 6)), 20, 8, 1, Jump::First),
            Some((0, 0))
        );
        assert_eq!(destination(None, 20, 8, 1, Jump::Last), Some((19, 7)));
        assert_eq!(destination(None, 0, 8, 1, Jump::Last), None);
        assert_eq!(destination(None, 20, 0, 1, Jump::First), None);
        assert_eq!(
            destination(Some((99, 99)), 20, 2, 1, Jump::RowEnd),
            Some((19, 1))
        );
    }
}

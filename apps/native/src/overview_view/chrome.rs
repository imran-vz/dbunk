//! Tool-document helpers shared by the captured tools (Administration,
//! History, CSV transfer, Backup / Restore, Compare schemas, Schema map and
//! the overview): connection context by display name, compact timestamps,
//! labelled detail parsing and check-box rows. Candidates to lift into
//! `ui/tool.rs`; they only style and format, callers keep handlers and AX.
use crate::style;
use dbunk_lib::backend::{DevelopmentConnection, DevelopmentEnvironment};
use gpui::{AnyElement, Div, ElementId, Role, SharedString, Stateful, div, prelude::*, px};

/// The connection a tool works on, by display name (never its id).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolConnection {
    pub engine: String,
    pub name: String,
    pub environment: DevelopmentEnvironment,
}

impl ToolConnection {
    /// The saved record for `id`, if the workspace has supplied it.
    pub fn find(records: &[DevelopmentConnection], id: Option<&str>) -> Option<Self> {
        let id = id?;
        records
            .iter()
            .find(|record| record.id == id)
            .map(|record| Self {
                engine: record.engine.clone(),
                name: single_line(&record.name, 128),
                environment: record.environment,
            })
    }

    /// Every saved record by id, for tools whose connection can change.
    pub fn index(records: &[DevelopmentConnection]) -> Vec<(String, Self)> {
        records
            .iter()
            .map(|record| {
                (
                    record.id.clone(),
                    Self {
                        engine: record.engine.clone(),
                        name: single_line(&record.name, 128),
                        environment: record.environment,
                    },
                )
            })
            .collect()
    }

    /// The indexed record for `id`.
    pub fn lookup<'a>(index: &'a [(String, Self)], id: Option<&str>) -> Option<&'a Self> {
        let id = id?;
        index
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, connection)| connection)
    }

    /// Engine logo, display name and environment tag for a `tool_header`.
    pub fn context(&self) -> AnyElement {
        crate::ui::tool_context(&self.engine, self.name.clone(), Some(self.environment))
            .into_any_element()
    }
}

/// Header context for a tool: the connection by name, or a quiet `dim`
/// placeholder while the workspace has not supplied the record yet. Never
/// shows a connection id.
pub(crate) fn connection_context(
    connection: Option<&ToolConnection>,
    bound: bool,
) -> Option<AnyElement> {
    match connection {
        Some(connection) => Some(connection.context()),
        None => Some(
            div()
                .text_color(style::dim())
                .whitespace_nowrap()
                .child(if bound {
                    "Saved connection"
                } else {
                    "No connection selected"
                })
                .into_any_element(),
        ),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Civil date (year, month, day) of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as i32, month, day)
}

/// Today's UTC date, for `compact_timestamp`.
pub(crate) fn today() -> (i32, u32, u32) {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    civil_from_days((seconds / 86_400) as i64)
}

/// A timestamp such as `2026-11-21T19:01:35.111720+00:00` (or with a space
/// separator) as `19:01:35` when it falls on `today`, `Nov 21, 19:01` in the
/// current year and `Nov 21 2025, 19:01` otherwise. Shown in the clock of
/// the stored value (no time-zone conversion). `None` when `raw` is not a
/// timestamp; callers then show it unchanged.
pub(crate) fn compact_timestamp(raw: &str, today: (i32, u32, u32)) -> Option<String> {
    let raw = raw.trim();
    let year: i32 = raw.get(0..4)?.parse().ok()?;
    if raw.get(4..5)? != "-" || raw.get(7..8)? != "-" {
        return None;
    }
    let month: u32 = raw.get(5..7)?.parse().ok()?;
    let day: u32 = raw.get(8..10)?.parse().ok()?;
    if !matches!(raw.get(10..11)?, "T" | " ") || raw.get(13..14)? != ":" {
        return None;
    }
    let hour: u32 = raw.get(11..13)?.parse().ok()?;
    let minute: u32 = raw.get(14..16)?.parse().ok()?;
    let second: Option<u32> = match raw.get(16..17) {
        Some(":") => Some(raw.get(17..19)?.parse().ok()?),
        _ => None,
    };
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 {
        return None;
    }
    let name = MONTHS[month as usize - 1];
    Some(if (year, month, day) == today {
        match second {
            Some(second) => format!("{hour:02}:{minute:02}:{second:02}"),
            None => format!("{hour:02}:{minute:02}"),
        }
    } else if year == today.0 {
        format!("{name} {day}, {hour:02}:{minute:02}")
    } else {
        format!("{name} {day} {year}, {hour:02}:{minute:02}")
    })
}

/// A timestamp in compact form with the full stored value as its tooltip.
pub(crate) fn timestamp(id: impl Into<ElementId>, raw: &str) -> Stateful<Div> {
    let shown = compact_timestamp(raw, today()).unwrap_or_else(|| raw.to_owned());
    div()
        .id(id)
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(shown)
        .tooltip(crate::ui::tooltip(raw.to_owned()))
        .tooltip_show_delay(crate::ui::tooltip_delay())
}

/// Collapses whitespace runs (SQL indentation, newlines) to single spaces
/// and keeps at most `max` characters, ending with `…` when clipped.
pub(crate) fn single_line(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max + 4));
    let mut count = 0;
    let mut clipped = false;
    for word in text.split_whitespace() {
        if count > 0 {
            if count >= max {
                clipped = true;
                break;
            }
            out.push(' ');
            count += 1;
        }
        for ch in word.chars() {
            if count >= max {
                clipped = true;
                break;
            }
            out.push(if ch.is_control() { ' ' } else { ch });
            count += 1;
        }
        if clipped {
            break;
        }
    }
    if clipped {
        out.push('…');
    }
    out
}

/// Splits a `a · b · c` summary row into exactly `columns` cells; the last
/// cell keeps any further separators (a query preview).
pub(crate) fn split_cells(label: &str, columns: usize) -> Vec<String> {
    let mut cells: Vec<String> = label
        .splitn(columns.max(1), " · ")
        .map(str::to_owned)
        .collect();
    cells.resize(columns.max(1), String::new());
    cells
}

/// True for a canonical hyphenated UUID (a connection or attempt id), which
/// resting UI never shows.
pub(crate) fn is_uuid(text: &str) -> bool {
    let bytes = text.trim().as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

/// `Label: value` lines of a captured-details text, plus the lines that
/// belong to no label (capture notes, caveats).
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Details {
    pub pairs: Vec<(String, String)>,
    pub notes: Vec<String>,
}

fn split_label(line: &str) -> Option<(&str, &str)> {
    let (label, value) = match line.strip_suffix(':') {
        Some(label) if !label.contains(':') => (label, ""),
        _ => line.split_once(": ")?,
    };
    let valid = !label.is_empty()
        && label.len() <= 64
        && label.chars().next().is_some_and(char::is_uppercase)
        && !label.contains(" · ")
        && !label.contains(':');
    valid.then_some((label, value))
}

/// Parses details written as `Label: value` lines. A line `Label:` takes
/// the next line as its value; a `Query…:` label takes every remaining line
/// (verbatim SQL). Other lines are notes.
pub(crate) fn parse_details(text: &str) -> Details {
    let mut details = Details::default();
    let mut open = false;
    let mut rest = false;
    for line in text.lines() {
        if rest || open {
            if let Some((_, value)) = details.pairs.last_mut() {
                if !value.is_empty() {
                    value.push('\n');
                }
                value.push_str(line);
            }
            open = false;
            continue;
        }
        match split_label(line) {
            Some((label, value)) => {
                let bare = value.is_empty() && line.ends_with(':');
                details.pairs.push((label.to_owned(), value.to_owned()));
                if bare {
                    if label.starts_with("Query") {
                        rest = true;
                    } else {
                        open = true;
                    }
                }
            }
            None => details.notes.push(line.to_owned()),
        }
    }
    details
}

/// Display form of a captured detail value: booleans as Yes/No, `NULL` and
/// empty values as absent, timestamps compact (the caller adds the full
/// value as a tooltip).
pub(crate) fn display_value(value: &str) -> Option<String> {
    match value.trim() {
        "" | "NULL" => None,
        "true" => Some("Yes".into()),
        "false" => Some("No".into()),
        value => Some(value.to_owned()),
    }
}

/// One `property_row` per detail pair. Timestamps show compact with the full
/// value in a tooltip, `Query` values are monospace and keep their lines,
/// ids (UUIDs) are left out of the resting view.
pub(crate) fn detail_rows(id: &'static str, details: &Details) -> Vec<Div> {
    let today = today();
    details
        .pairs
        .iter()
        .enumerate()
        .filter(|(_, (_, value))| !is_uuid(value))
        .map(|(index, (label, value))| {
            let shown = display_value(value);
            let element = match shown {
                None => div()
                    .text_color(style::faint())
                    .child("—")
                    .into_any_element(),
                Some(shown) if label.starts_with("Query") => div()
                    .font_family(style::MONO)
                    .text_size(px(style::FONT_DATA))
                    .whitespace_normal()
                    .child(shown)
                    .into_any_element(),
                Some(shown) if compact_timestamp(&shown, today).is_some() => {
                    timestamp((id, index), &shown).into_any_element()
                }
                Some(shown) => div().whitespace_normal().child(shown).into_any_element(),
            };
            crate::ui::property_row(label.clone(), element)
        })
        .collect()
}

/// An on/off option as a check-box row (D6): `CONTROL` high, 12 pt box and
/// a `FONT` label. Role `CheckBox` with its toggled state; callers add focus
/// tracking, `on_click` and AX actions (and must set toggled again if they
/// install their own `a11y_synthetic_children`).
pub(crate) fn check_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    enabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    div()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(label.clone())
        .aria_toggled(gpui::accesskit::Toggled::from(checked))
        .flex_none()
        .h(px(style::CONTROL))
        .px(px(style::S3))
        .flex()
        .items_center()
        .gap(px(style::S3))
        .rounded(px(style::R_SM))
        .text_sm()
        .whitespace_nowrap()
        .text_color(if enabled {
            style::text()
        } else {
            style::faint()
        })
        .when(!enabled, |row| row.opacity(0.5))
        .when(enabled, |row| {
            row.cursor_pointer().hover(|s| s.bg(style::hover()))
        })
        .focus(|s| s.shadow(vec![crate::ui::ring()]))
        .child(crate::ui::check_box(checked).mt(px(0.)))
        .child(label)
}

/// A quiet one-line note in `dim` under the control it qualifies.
pub(crate) fn note(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(style::FONT_SMALL))
        .text_color(style::dim())
        .child(text.into())
}

/// A column of tool content: at most `TOOL_COLUMN` wide with `S5` between
/// groups.
pub(crate) fn tool_column() -> Div {
    div()
        .w_full()
        .max_w(px(style::TOOL_COLUMN))
        .flex()
        .flex_col()
        .gap(px(style::S5))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TODAY: (i32, u32, u32) = (2026, 11, 21);

    #[test]
    fn timestamps_are_compact_and_reject_other_text() {
        assert_eq!(
            compact_timestamp("2026-11-21T19:01:35.111720+00:00", TODAY).as_deref(),
            Some("19:01:35")
        );
        assert_eq!(
            compact_timestamp("2026-11-20 08:05:00+00", TODAY).as_deref(),
            Some("Nov 20, 08:05")
        );
        assert_eq!(
            compact_timestamp("2025-01-02T03:04", TODAY).as_deref(),
            Some("Jan 2 2025, 03:04")
        );
        assert_eq!(
            compact_timestamp("2026-11-21T19:01", TODAY).as_deref(),
            Some("19:01")
        );
        assert_eq!(compact_timestamp("active", TODAY), None);
        assert_eq!(compact_timestamp("2026-13-01T00:00", TODAY), None);
        assert_eq!(compact_timestamp("2026-11-21T25:00", TODAY), None);
        assert_eq!(compact_timestamp("2026-11-21", TODAY), None);
        assert_eq!(compact_timestamp("1234567", TODAY), None);
    }

    #[test]
    fn civil_dates_follow_the_gregorian_calendar() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(20_779), (2026, 11, 22));
    }

    #[test]
    fn single_line_collapses_whitespace_and_clips_by_characters() {
        assert_eq!(
            single_line("SELECT a\n      FROM  t\tWHERE x", 80),
            "SELECT a FROM t WHERE x"
        );
        assert_eq!(single_line("äöüß äöüß", 4), "äöüß…");
        assert_eq!(single_line("  ", 10), "");
        assert_eq!(single_line("abc", 3), "abc");
    }

    #[test]
    fn summary_rows_split_into_fixed_cells() {
        assert_eq!(
            split_cells(
                "12 · dbunk · active · Client / ClientRead · SELECT 1 · 2",
                5
            ),
            vec![
                "12",
                "dbunk",
                "active",
                "Client / ClientRead",
                "SELECT 1 · 2"
            ]
        );
        assert_eq!(split_cells("61", 3), vec!["61", "", ""]);
    }

    #[test]
    fn uuids_are_recognised() {
        assert!(is_uuid("47c1c5c4-4ade-4301-a1a6-b60b8791cead"));
        assert!(!is_uuid("billing-dev"));
        assert!(!is_uuid("47c1c5c4x4ade-4301-a1a6-b60b8791cead"));
    }

    #[test]
    fn details_parse_into_pairs_notes_and_verbatim_query() {
        let text = "Database d · reader PID 1 · collected a to b. Readings may change\nPID: 12\nApplication: \nServer version:\n16.2\nQuery:\nSELECT 1\n  FROM t\nUser: not a label";
        let details = parse_details(text);
        assert_eq!(
            details.notes,
            vec!["Database d · reader PID 1 · collected a to b. Readings may change"]
        );
        assert_eq!(
            details.pairs,
            vec![
                ("PID".into(), "12".into()),
                ("Application".into(), "".into()),
                ("Server version".into(), "16.2".into()),
                (
                    "Query".into(),
                    "SELECT 1\n  FROM t\nUser: not a label".into()
                ),
            ]
        );
    }

    #[test]
    fn display_values_read_as_words() {
        assert_eq!(display_value("true").as_deref(), Some("Yes"));
        assert_eq!(display_value("false").as_deref(), Some("No"));
        assert_eq!(display_value("NULL"), None);
        assert_eq!(display_value(" "), None);
        assert_eq!(display_value("idle").as_deref(), Some("idle"));
    }
}

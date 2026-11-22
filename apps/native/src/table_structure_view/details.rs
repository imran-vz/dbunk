//! Presentation of the Structure page: the selected row's properties for
//! the Details panel, and the content-fitted Columns layout. The exact
//! catalog text stays available, unchanged, behind the Diagnostics
//! disclosure.
use crate::{
    structure_table::{ColumnSpec, Shown, clip},
    table_structure_model::{Capture, Section},
};
use gpui::SharedString;

/// One `ui::property_list` row; `None` renders the absent-value dash.
pub(super) type Property = (SharedString, Option<SharedString>);

/// Rows sampled when fitting the Columns layout.
const FIT_ROWS: usize = 200;
/// Name, Type and Default fit their content between these bounds.
const FIT_MIN: f32 = 64.;
const FIT_MAX: f32 = 220.;

/// Labels that only matter for diagnostics: object ids, capture time and
/// internal numbering. They stay in the raw text.
fn diagnostic(label: &str) -> bool {
    label.contains("OID")
        || matches!(
            label,
            "Captured" | "Attribute number" | "Server version number"
        )
}

fn friendly_label(label: &str) -> &str {
    match label {
        "Column" => "Name",
        "Data type" => "Type",
        "Default / generated expression" => "Default",
        "Primary-key position" => "Primary key position",
        other => other,
    }
}

/// `"id"` → `id` when the identifier needs no inner quotes; booleans read as
/// Yes/No. The value is one bounded display line.
fn friendly_value(value: &str) -> SharedString {
    let shown = match value {
        "true" => "Yes".to_owned(),
        "false" => "No".to_owned(),
        _ => value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .filter(|inner| !inner.contains('"'))
            .unwrap_or(value)
            .to_owned(),
    };
    clip(&shown).into()
}

/// `Label (text, N UTF-8 bytes):` → `(label, N)`.
fn text_header(line: &str) -> Option<(&str, usize)> {
    let head = line.strip_suffix(" UTF-8 bytes):")?;
    let (label, bytes) = head.rsplit_once(" (text, ")?;
    Some((label, bytes.parse().ok()?))
}

/// `Label: value` with a short capitalised label.
fn pair(line: &str) -> Option<(&str, &str)> {
    let (label, value) = line.split_once(": ")?;
    (label.len() <= 64 && label.starts_with(|c: char| c.is_ascii_uppercase()))
        .then_some((label, value))
}

/// `1. item` from an ordered list in the details text.
fn list_item(line: &str) -> Option<&str> {
    let (number, item) = line.split_once(". ")?;
    (!number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())).then_some(item)
}

fn push(out: &mut Vec<Property>, label: &str, value: Option<&str>) {
    if diagnostic(label) {
        return;
    }
    out.push((
        SharedString::from(friendly_label(label).to_owned()),
        value.map(friendly_value),
    ));
}

/// The selected row's properties, read from the exact details text: its
/// three header lines (section, ids, capture time) are skipped, text values
/// are taken by their declared byte length, and trailing prose after the
/// properties is left to the raw view.
pub(super) fn properties(details: &str) -> Vec<Property> {
    let mut out = Vec::new();
    let mut rest = details;
    let mut header = 0;
    while !rest.is_empty() {
        let (line, after) = rest.split_once('\n').unwrap_or((rest, ""));
        rest = after;
        if header < 3 {
            header += 1;
            continue;
        }
        if line.is_empty() {
            if out.is_empty() {
                continue;
            }
            break;
        }
        if let Some((label, bytes)) = text_header(line) {
            // A length that does not fit the text falls back to the rest.
            let value = rest
                .get(..bytes)
                .unwrap_or_else(|| rest.trim_end_matches('\n'));
            rest = rest.get(bytes..).unwrap_or("");
            rest = rest.strip_prefix('\n').unwrap_or(rest);
            push(&mut out, label, Some(value));
        } else if let Some(item) = list_item(line) {
            // Items of the list heading just before them, comma-joined.
            if let Some((_, value)) = out.last_mut() {
                let item = friendly_value(item);
                *value = Some(match value.take() {
                    Some(joined) if !joined.is_empty() => format!("{joined}, {item}").into(),
                    _ => item,
                });
            }
        } else if let Some((label, value)) = pair(line) {
            match value {
                "NULL (absent)" => push(&mut out, label, None),
                "empty text (0 UTF-8 bytes)" => push(&mut out, label, Some("Empty string")),
                value => push(&mut out, label, Some(value)),
            }
        } else if let Some(label) = line.strip_suffix(':') {
            // A list heading ("Ordered key columns:"); items follow.
            if !diagnostic(label) {
                out.push((
                    SharedString::from(friendly_label(label).to_owned()),
                    Some(SharedString::default()),
                ));
            }
        }
    }
    // A list heading with no items shows the absent dash.
    for (_, value) in &mut out {
        if value.as_ref().is_some_and(|value| value.is_empty()) {
            *value = None;
        }
    }
    out
}

/// `Updated 19:01` from a capture timestamp (`2026-11-21 19:01:58.88+00`);
/// the full value goes in the tooltip. `None` for an unexpected shape.
pub(super) fn updated_label(captured: &str) -> Option<String> {
    let (_, time) = captured.split_once([' ', 'T'])?;
    let clock = time.get(..5)?;
    let bytes = clock.as_bytes();
    (bytes[2] == b':'
        && [0, 1, 3, 4]
            .into_iter()
            .all(|index| bytes[index].is_ascii_digit()))
    .then(|| format!("Updated {clock}"))
}

/// The Columns section layout: Name, Type and Default fit their content
/// (at most 220 pt), Nullable stays 72, Keys keeps its chips, Comment takes
/// what is left. Other sections keep their static layout.
pub(super) fn fitted_columns(capture: &Capture) -> Vec<ColumnSpec> {
    let specs = Section::Columns.columns();
    let rows = (0..capture.count(Section::Columns).min(FIT_ROWS))
        .filter_map(|index| capture.cells(Section::Columns, index))
        .collect::<Vec<_>>();
    specs
        .iter()
        .enumerate()
        .map(|(column, spec)| match spec.title {
            "Name" | "Type" | "Default" => {
                let samples = rows.iter().filter_map(|row| match row.get(column) {
                    Some(Shown::Text(text) | Shown::Faint(text)) => Some(text.as_str()),
                    _ => None,
                });
                ColumnSpec {
                    width: crate::ui::fit_track(spec.title, samples, spec.mono, FIT_MIN, FIT_MAX),
                    fill: false,
                    ..*spec
                }
            }
            _ => *spec,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(properties: &[Property]) -> Vec<(&str, Option<&str>)> {
        properties
            .iter()
            .map(|(label, value)| (label.as_str(), value.as_ref().map(SharedString::as_str)))
            .collect()
    }

    #[test]
    fn column_details_become_readable_properties_without_ids() {
        let details = "Columns · \"billing\".\"invoices\"\n\
            Database OID: 16384; relation OID: 16889\n\
            Captured: 2026-11-21 19:01:58.88166+00\n\
            \n\
            Column: \"id\"\n\
            Attribute number: 1\n\
            Nullable: false\n\
            Identity: GENERATED ALWAYS\n\
            Generated: None\n\
            Data type (text, 6 UTF-8 bytes):\nbigint\n\
            Default / generated expression: NULL (absent)\n\
            Comment: empty text (0 UTF-8 bytes)\n\
            Primary-key position: 1\n\
            Collation name (text, 3 UTF-8 bytes):\na\nb\n";
        assert_eq!(
            labels(&properties(details)),
            [
                ("Name", Some("id")),
                ("Nullable", Some("No")),
                ("Identity", Some("GENERATED ALWAYS")),
                ("Generated", Some("None")),
                ("Type", Some("bigint")),
                ("Default", None),
                ("Comment", Some("Empty string")),
                ("Primary key position", Some("1")),
                // Multi-line text is one bounded display line.
                ("Collation name", Some("a b")),
            ]
        );
    }

    #[test]
    fn text_values_are_taken_by_length_even_when_they_look_like_labels() {
        let details = "Indexes · \"s\".\"t\"\nids\nCaptured: x\n\n\
            Definition (text, 10 UTF-8 bytes):\nKind: fake\n\
            Ordered key columns:\n1. \"a\"\n2. \"b\"\n\
            Included columns:\n\
            \n\
            Trailing prose: ignored\n";
        assert_eq!(
            labels(&properties(details)),
            [
                ("Definition", Some("Kind: fake")),
                ("Ordered key columns", Some("a, b")),
                ("Included columns", None),
            ]
        );
    }

    #[test]
    fn updated_label_shows_the_clock_time_only() {
        assert_eq!(
            updated_label("2026-11-21 19:01:58.88166+00").as_deref(),
            Some("Updated 19:01")
        );
        assert_eq!(
            updated_label("2026-11-21T07:05:00Z").as_deref(),
            Some("Updated 07:05")
        );
        assert_eq!(updated_label("yesterday"), None);
        assert_eq!(updated_label("2026-11-21 1:2"), None);
        assert_eq!(updated_label("2026-11-21 ab:cd"), None);
    }

    #[test]
    fn malformed_text_lengths_never_panic() {
        let details = "a\nb\nc\n\nComment (text, 99 UTF-8 bytes):\nshort\n";
        assert_eq!(labels(&properties(details)), [("Comment", Some("short"))]);
        let details = "a\nb\nc\n\nComment (text, 1 UTF-8 bytes):\né\n";
        // A length that splits a character falls back to the remainder.
        assert_eq!(properties(details).len(), 1);
    }
}

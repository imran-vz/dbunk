//! Drizzle-style Structure tables shared by every engine: bounded one-line
//! display cells, section column layouts and their rendering. PostgreSQL's
//! Structure page virtualizes these lines; the other engines' structures are
//! small and render each section whole.
use crate::style;
use gpui::{AnyElement, Div, Role, SharedString, div, prelude::*, px};

/// Display cells keep at most this many characters before an ellipsis.
pub const CELL_CHARS: usize = 160;
/// Every table line: section titles, column titles and rows.
pub const LINE: f32 = 28.;
/// Horizontal inset of the section tables inside the page.
pub const INSET: f32 = 16.;
/// Narrowest a section column shrinks to before its text truncates away.
const MIN_CELL: f32 = 32.;

/// One column of a section table. `fill` columns share the spare width and
/// never shrink below `width`; the others are exactly `width` pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnSpec {
    pub title: &'static str,
    pub width: f32,
    pub fill: bool,
    pub mono: bool,
}
pub const fn fixed(title: &'static str, width: f32) -> ColumnSpec {
    ColumnSpec {
        title,
        width,
        fill: false,
        mono: false,
    }
}
pub const fn fill(title: &'static str, width: f32) -> ColumnSpec {
    ColumnSpec {
        title,
        width,
        fill: true,
        mono: false,
    }
}
pub const fn mono(spec: ColumnSpec) -> ColumnSpec {
    ColumnSpec { mono: true, ..spec }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Keys and identity: primary key, unique, foreign key.
    Key,
    Plain,
    /// States that need attention: invalid, disabled, not validated.
    Warn,
}
/// Cell text is shared, so rendering a cached row only bumps refcounts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub text: SharedString,
    pub tone: Tone,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shown {
    Text(SharedString),
    /// Absent, default or otherwise low-signal values.
    Faint(SharedString),
    Tags(Vec<Tag>),
}

/// One line, at most `CELL_CHARS` characters. Line breaks and tabs collapse
/// to single spaces; other control characters show as U+FFFD.
pub fn clip(value: &str) -> String {
    clip_chars(value.chars(), value.len())
}
/// `clip` over a lazy sequence, so a prefixed value is never copied whole.
pub fn clip_chars(value: impl Iterator<Item = char>, len: usize) -> String {
    let mut out = String::with_capacity(len.min(CELL_CHARS * 4));
    let mut chars = 0;
    for c in value {
        if chars == CELL_CHARS {
            out.push('…');
            break;
        }
        if matches!(c, '\n' | '\r' | '\t') {
            if !out.ends_with(' ') {
                out.push(' ');
                chars += 1;
            }
            continue;
        }
        out.push(if c.is_control() { '\u{FFFD}' } else { c });
        chars += 1;
    }
    out
}
pub fn text(value: String) -> Shown {
    Shown::Text(value.into())
}
pub fn faint(value: &str) -> Shown {
    Shown::Faint(SharedString::new(value))
}
/// The 1-based `#` cell.
pub fn ordinal(index: usize) -> Shown {
    faint(&(index + 1).to_string())
}
/// A clipped value; empty shows a faint dash.
pub fn or_dash(value: &str) -> Shown {
    if value.is_empty() {
        faint("—")
    } else {
        text(clip(value))
    }
}
/// A foreign-key action; the engine's default action is faint.
pub fn referential(label: &str, default: bool) -> Shown {
    if default {
        faint(label)
    } else {
        text(clip(label))
    }
}
/// `PK`, numbered by key position when the key spans several columns.
pub fn key_tag(position: usize, composite: bool) -> Tag {
    if composite {
        tag(&format!("PK {position}"), Tone::Key)
    } else {
        tag("PK", Tone::Key)
    }
}
/// A clipped value; absent shows a faint dash and empty a faint `''`.
pub fn optional(value: &Option<String>) -> Shown {
    match value {
        Some(value) if value.is_empty() => faint("''"),
        Some(value) => text(clip(value)),
        None => faint("—"),
    }
}
pub fn yes_no(value: bool) -> Shown {
    if value {
        text("yes".into())
    } else {
        faint("no")
    }
}
pub fn nullable(value: bool) -> Shown {
    if value {
        faint("NULL")
    } else {
        text("NOT NULL".into())
    }
}
pub fn tag(text: &str, tone: Tone) -> Tag {
    Tag {
        text: SharedString::new(text),
        tone,
    }
}
/// The present tags, in order.
pub fn tags(items: impl IntoIterator<Item = Option<Tag>>) -> Shown {
    Shown::Tags(items.into_iter().flatten().collect())
}

/// The Columns layout of engines without per-column comments.
pub const KEYED_COLUMNS: &[ColumnSpec] = &[
    mono(fixed("#", 40.)),
    fill("Name", 140.),
    mono(fill("Type", 120.)),
    mono(fill("Default", 120.)),
    fixed("Nullable", 84.),
    fixed("Keys", 150.),
];

/// A whole section for engines that render every row at once. Built once
/// when the structure loads; rendering only clones shared text.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionTable {
    pub title: &'static str,
    pub columns: &'static [ColumnSpec],
    /// One cell per column in each row.
    rows: Vec<Vec<Shown>>,
    /// Each row's AX name.
    labels: Vec<SharedString>,
    /// Shown beside the title when there are no rows.
    pub empty: &'static str,
}

impl SectionTable {
    pub fn new(
        title: &'static str,
        columns: &'static [ColumnSpec],
        rows: Vec<Vec<Shown>>,
        empty: &'static str,
    ) -> Self {
        debug_assert!(rows.iter().all(|row| row.len() == columns.len()), "{title}");
        let labels = rows.iter().map(|row| row_label(row).into()).collect();
        Self {
            title,
            columns,
            rows,
            labels,
            empty,
        }
    }
    #[cfg(test)]
    pub fn rows(&self) -> &[Vec<Shown>] {
        &self.rows
    }
}

/// One cell of a section line. `width` is the preferred size, not a floor:
/// every column shrinks (and truncates) so none is pushed past a narrow page.
pub fn cell(spec: &ColumnSpec, cell: Option<Shown>) -> AnyElement {
    let base = div()
        .h_full()
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(4.))
        .overflow_hidden()
        .whitespace_nowrap()
        .flex_basis(px(spec.width))
        .flex_shrink_1()
        .min_w(px(spec.width.min(MIN_CELL)))
        .when(spec.fill, |cell| cell.flex_grow_1())
        .when(!spec.fill, |cell| cell.flex_grow_0())
        .when(spec.mono, |cell| cell.font_family(style::MONO));
    match cell {
        Some(Shown::Text(text)) => base.child(div().min_w_0().truncate().child(text)),
        Some(Shown::Faint(text)) => base
            .text_color(style::faint())
            .child(div().min_w_0().truncate().child(text)),
        Some(Shown::Tags(tags)) => base.children(tags.into_iter().map(|tag| {
            let (fill, color) = match tag.tone {
                Tone::Key => (style::primary_fill(), style::accent()),
                Tone::Warn => (style::warn_fill(), style::warn()),
                Tone::Plain => (style::raised(), style::dim()),
            };
            div()
                .flex_none()
                .h(px(16.))
                .px(px(5.))
                .flex()
                .items_center()
                .rounded(px(3.))
                .bg(fill)
                .font_family(style::MONO)
                .text_size(px(style::FONT_SMALL))
                .text_color(color)
                .child(tag.text)
        })),
        None => base,
    }
    .into_any_element()
}

/// The bordered card segment every table line sits in.
pub fn card(top: bool, bottom: bool) -> Div {
    div()
        .h_full()
        .flex()
        .items_center()
        .border_l_1()
        .border_r_1()
        .border_t_1()
        .border_color(style::line_soft())
        .when(top, |card| card.rounded_t(px(6.)))
        .when(bottom, |card| card.border_b_1().rounded_b(px(6.)))
}

/// An empty page line, inset like the section tables.
pub fn line() -> Div {
    div().w_full().h(px(LINE)).px(px(INSET))
}

/// A section title with its row count; an empty section says why, faintly.
pub fn title_line(label: impl Into<SharedString>, count: usize, empty: Option<&str>) -> Div {
    line()
        .flex()
        .items_end()
        .gap(px(8.))
        .pb(px(5.))
        .child(
            div()
                .flex_none()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if count == 0 {
                    style::faint()
                } else {
                    style::text()
                })
                .child(label.into()),
        )
        .child(
            div()
                .flex_none()
                .font_family(style::MONO)
                .text_size(px(style::FONT_SMALL))
                .text_color(style::faint())
                .child(count.to_string()),
        )
        .when_some(empty.filter(|_| count == 0), |line, empty| {
            line.child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(style::faint())
                    .child(empty.to_owned()),
            )
        })
}

/// The column titles card at the top of a section table.
pub fn head_line(columns: &[ColumnSpec]) -> Div {
    line().child(
        card(true, columns.is_empty())
            .bg(style::panel())
            .text_size(px(style::FONT_SMALL))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(style::faint())
            .children(columns.iter().map(|spec| {
                cell(
                    &ColumnSpec {
                        mono: false,
                        ..*spec
                    },
                    Some(Shown::Text(SharedString::new_static(spec.title))),
                )
            })),
    )
}

/// A row's AX name: its non-empty text cells, comma-joined.
pub fn row_label(cells: &[Shown]) -> String {
    cells
        .iter()
        .filter_map(|cell| match cell {
            Shown::Text(text) | Shown::Faint(text) => Some(text.to_string()),
            Shown::Tags(tags) if !tags.is_empty() => Some(
                tags.iter()
                    .map(|tag| tag.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            Shown::Tags(_) => None,
        })
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every section of a small structure, top to bottom. Empty sections keep
/// their title so the page always has the same outline.
pub fn sections(
    id: &'static str,
    label: impl Into<SharedString>,
    tables: &[SectionTable],
) -> AnyElement {
    let mut page = div()
        .id(id)
        .role(Role::Group)
        .aria_label(label.into())
        .flex()
        .flex_col()
        .pt(px(4.))
        .pb(px(12.));
    for (section, table) in tables.iter().enumerate() {
        let count = table.rows.len();
        page = page.child(title_line(table.title, count, Some(table.empty)));
        if count == 0 {
            continue;
        }
        page =
            page.child(head_line(table.columns)).child(
                div()
                    .id((id, section))
                    .role(Role::List)
                    .aria_label(SharedString::from(format!("{} {count}", table.title)))
                    .flex()
                    .flex_col()
                    .children(table.rows.iter().zip(&table.labels).enumerate().map(
                        |(index, (cells, label))| {
                            line()
                                .id(index)
                                .role(Role::ListItem)
                                .aria_label(label.clone())
                                .group("structure-table-row")
                                .child(
                                    card(false, index + 1 == count)
                                        .text_color(style::text())
                                        .bg(style::bg())
                                        .group_hover("structure-table-row", |s| {
                                            s.bg(style::row_hover())
                                        })
                                        .children(
                                            table.columns.iter().zip(cells).map(|(spec, shown)| {
                                                cell(spec, Some(shown.clone()))
                                            }),
                                        ),
                                )
                        },
                    )),
            );
    }
    page.into_any_element()
}

/// Read-only definition text under the section tables, inset like them.
pub fn definition(id: &'static str, title: &'static str, sql: SharedString) -> Div {
    div()
        .flex()
        .flex_col()
        .child(title_line(title, 1, None))
        .child(
            div().px(px(INSET)).child(
                div()
                    .id(id)
                    .role(Role::Document)
                    .aria_label(SharedString::from(title))
                    .p(px(8.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(style::line_soft())
                    .bg(style::panel())
                    .font_family(style::MONO)
                    .text_color(style::text())
                    .whitespace_normal()
                    .child(sql),
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_stay_on_one_bounded_line() {
        assert_eq!(clip("a\r\n\tb"), "a b");
        assert_eq!(clip("x\u{7}y"), "x\u{FFFD}y");
        let long = clip(&"é".repeat(CELL_CHARS * 3));
        assert_eq!(long.chars().count(), CELL_CHARS + 1);
        assert!(long.ends_with('…'));
        assert_eq!(clip(&"z".repeat(CELL_CHARS)), "z".repeat(CELL_CHARS));
    }

    #[test]
    fn row_labels_skip_empty_cells_and_name_tags() {
        let cells = [
            text("id".into()),
            faint(""),
            tags([Some(tag("PK", Tone::Key)), None, Some(tag("FK", Tone::Key))]),
            tags([None]),
            nullable(false),
        ];
        assert_eq!(row_label(&cells), "id, PK FK, NOT NULL");
    }

    #[test]
    fn optional_values_distinguish_absent_from_empty() {
        assert_eq!(optional(&None), Shown::Faint("—".into()));
        assert_eq!(optional(&Some(String::new())), Shown::Faint("''".into()));
        assert_eq!(optional(&Some("0".into())), Shown::Text("0".into()));
    }
}

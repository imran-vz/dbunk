//! Tool-document scaffold: one header, one table, one property list, one
//! empty state and one caveat hint, shared by Administration, Schema map,
//! Compare schemas, CSV transfer, Backup / Restore, History and the other
//! tools. Also the one environment tag. Helpers only style; callers keep
//! focus handles, listeners and AX actions.
// Views migrate onto these primitives in round 1 (phase B); remove this allow
// once every primitive has a caller.
use super::{list_row_fill, ring, tooltip, tooltip_delay};
use crate::style;
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::{
    AnyElement, Div, ElementId, Role, SharedString, Stateful, div, img, prelude::*, px, svg,
};

/// Most actions a `tool_header` shows; the rest go in its `⋯` menu.
pub const TOOL_HEADER_ACTIONS: usize = 4;

/// A tool document's header row: `TOOLBAR` high, title in `FONT` semibold
/// `text`, then `context` (usually `tool_context`: engine logo, connection
/// name and `env_tag`, never an id), then up to `TOOL_HEADER_ACTIONS`
/// right-aligned actions. Callers pass extra actions through their own `⋯`
/// overflow button as the last action; a debug build asserts the limit.
pub fn tool_header(
    title: impl Into<SharedString>,
    context: Option<AnyElement>,
    actions: Vec<AnyElement>,
) -> Div {
    debug_assert!(
        actions.len() <= TOOL_HEADER_ACTIONS + 1,
        "tool_header shows at most {TOOL_HEADER_ACTIONS} actions plus an overflow button"
    );
    div()
        .flex_none()
        .h(px(style::TOOLBAR))
        .px(px(style::INSET_CONTENT))
        .flex()
        .flex_nowrap()
        .items_center()
        .gap(px(style::S4))
        .overflow_hidden()
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .whitespace_nowrap()
        .child(
            div()
                .flex_none()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(style::text())
                .child(title.into()),
        )
        .when_some(context, |header, context| {
            header.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .child(context),
            )
        })
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(style::S2))
                .children(actions),
        )
}

/// Engine logo (or badge), connection display name and the environment tag
/// for a `tool_header` or dialog header. `name` is the display name, never a
/// connection id.
pub fn tool_context(
    engine: &str,
    name: impl Into<SharedString>,
    environment: Option<DevelopmentEnvironment>,
) -> Div {
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(style::S3))
        .text_color(style::dim())
        .child(engine_mark(engine))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(name.into()),
        )
        .when_some(environment, |context, environment| {
            context.child(env_tag(environment))
        })
}

/// A 14 pt engine logo, or the engine's colour badge when it has none.
pub fn engine_mark(engine: &str) -> AnyElement {
    match style::engine_logo(engine) {
        Some(logo) => img(logo).size(px(14.)).flex_none().into_any_element(),
        None => {
            let (badge, color) = style::engine_badge(engine);
            div()
                .size(px(14.))
                .flex_none()
                .rounded(px(style::R_SM - 1.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(8.))
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(style::bg())
                .bg(style::with_alpha(color, 0xff))
                .child(badge)
                .into_any_element()
        }
    }
}

/// The one environment tag: filled in the environment colour, `R_SM`,
/// 10 pt semibold uppercase short name (DEV, TEST, STAGE, PROD) in dark
/// text. Status bar, dialog headers, sidebar group headers and palette rows;
/// everywhere else write the full name (`style::env_name`).
pub fn env_tag(environment: DevelopmentEnvironment) -> Div {
    div()
        .flex_none()
        .h(px(16.))
        .px(px(style::S2 + 1.))
        .flex()
        .items_center()
        .rounded(px(style::R_SM))
        .bg(style::with_alpha(style::env(Some(environment)), 0xff))
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::on_accent())
        .whitespace_nowrap()
        .child(style::env_short(environment))
}

/// A 6 pt dot in the environment colour, for group headers beside a
/// sentence-case full name.
pub fn env_dot(environment: Option<DevelopmentEnvironment>) -> Div {
    div()
        .flex_none()
        .size(px(6.))
        .rounded_full()
        .bg(style::with_alpha(style::env(environment), 0xff))
}

/// How a `data_table` column takes width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Track {
    /// A fixed width in points (use `fit_track` to size it from content).
    Fixed(f32),
    /// Takes the remaining width and ellipsizes. Use for the last column.
    Flex,
}

/// One `data_table` column.
#[derive(Clone, Debug, PartialEq)]
pub struct TableColumn {
    /// Sentence-case header text.
    pub label: SharedString,
    pub track: Track,
    /// Right-align (numbers, durations).
    pub end: bool,
    /// Monospace cells (SQL, identifiers, hosts, in-row durations).
    pub mono: bool,
}

impl TableColumn {
    pub fn fixed(label: impl Into<SharedString>, width: f32) -> Self {
        Self {
            label: label.into(),
            track: Track::Fixed(width),
            end: false,
            mono: false,
        }
    }

    pub fn flex(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            track: Track::Flex,
            end: false,
            mono: false,
        }
    }

    /// Right-aligned cells and header.
    pub fn end(mut self) -> Self {
        self.end = true;
        self
    }

    /// Monospace cells.
    pub fn mono(mut self) -> Self {
        self.mono = true;
        self
    }
}

/// Approximate advance of one character of 12 pt UI text and of 12 pt
/// monospace data, in points. Used only to size columns before layout.
const SANS_ADVANCE: f32 = 6.6;
const MONO_ADVANCE: f32 = 7.2;
/// Horizontal padding of a table cell on each side.
const CELL_PAD: f32 = style::S3;

/// A fixed column width that fits `label` and the widest of `samples`
/// (character count times an average advance, plus cell padding), clamped
/// to `min..=max`. Pure: call it once per data change, not per frame.
pub fn fit_track<'a>(
    label: &str,
    samples: impl IntoIterator<Item = &'a str>,
    mono: bool,
    min: f32,
    max: f32,
) -> f32 {
    let advance = if mono { MONO_ADVANCE } else { SANS_ADVANCE };
    let widest = samples
        .into_iter()
        .map(|sample| sample.chars().count() as f32 * advance)
        .fold(label.chars().count() as f32 * SANS_ADVANCE, f32::max);
    (widest + CELL_PAD * 2.).clamp(min, max.max(min))
}

fn cell_frame(column: &TableColumn) -> Div {
    let cell = div()
        .h_full()
        .px(px(CELL_PAD))
        .flex()
        .items_center()
        .overflow_hidden()
        .whitespace_nowrap()
        .when(column.end, |cell| cell.justify_end());
    match column.track {
        Track::Fixed(width) => cell.flex_none().w(px(width)),
        Track::Flex => cell.flex_1().min_w_0(),
    }
}

/// The `GRID_ROW` header of a `data_table`: `FONT_SMALL` semibold `dim`,
/// sentence case, `line_soft` rule underneath.
pub fn data_table_header(columns: &[TableColumn]) -> Div {
    div()
        .flex_none()
        .h(px(style::GRID_ROW))
        .px(px(style::S2))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(style::line_soft())
        .text_size(px(style::FONT_SMALL))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .text_color(style::dim())
        .children(columns.iter().map(|column| {
            cell_frame(column).child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(column.label.clone()),
            )
        }))
}

/// One text cell of a `data_table` row, styled for its column. `None`
/// renders an em dash in `faint` (an absent value, not data).
pub fn data_cell(column: &TableColumn, value: Option<SharedString>) -> Div {
    let frame = cell_frame(column);
    match value {
        Some(value) => frame.child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .when(column.end, |text| text.text_right())
                .when(column.mono, |text| {
                    text.font_family(style::MONO)
                        .text_size(px(style::FONT_DATA))
                })
                .child(value),
        ),
        None => frame.text_color(style::faint()).child("—"),
    }
}

/// A `data_table` body row: `GRID_ROW` high, `line_soft` separator,
/// `list_row` hover and selection fills (`select` in the focused pane,
/// `raised` otherwise). `cells` come from `data_cell` (or custom elements
/// wrapped the same way), one per column. Sets `.hover(...)`; callers add
/// `on_click`, focus and AX.
pub fn data_table_row(
    id: impl Into<ElementId>,
    cells: impl IntoIterator<Item = Div>,
    selected: bool,
    focused: bool,
) -> Stateful<Div> {
    let fill = list_row_fill(selected, focused);
    div()
        .id(id)
        .role(Role::Row)
        .flex_none()
        .h(px(style::GRID_ROW))
        .px(px(style::S2))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(style::line_soft())
        .text_sm()
        .text_color(style::text())
        .when_some(fill, |row, fill| row.bg(fill))
        .when(fill.is_none(), |row| {
            row.hover(|s| s.bg(style::row_hover()))
        })
        .focus(|s| s.shadow(vec![ring()]))
        .children(cells)
}

/// One `property_list` row: a 120 pt `dim` label and a value element. Use
/// `ui::mono` for identifier values.
pub fn property_row(label: impl Into<SharedString>, value: impl IntoElement) -> Div {
    div()
        .flex_none()
        .min_h(px(style::GRID_ROW))
        .py(px(style::S1))
        .flex()
        .items_start()
        .gap(px(style::S4))
        .text_sm()
        .child(
            div()
                .flex_none()
                .w(px(style::PROPERTY_LABEL))
                .pt(px(1.))
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_color(style::dim())
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .pt(px(1.))
                .text_color(style::text())
                .child(value),
        )
}

/// Labelled values for admin details, the structure Details panel and job
/// details: one `property_row` per `(label, value)` pair, values in sans
/// `text`. Absent values show an em dash in `faint`.
pub fn property_list(pairs: impl IntoIterator<Item = (SharedString, Option<SharedString>)>) -> Div {
    div()
        .flex()
        .flex_col()
        .children(pairs.into_iter().map(|(label, value)| match value {
            Some(value) => property_row(label, div().child(value)),
            None => property_row(label, div().text_color(style::faint()).child("—")),
        }))
}

/// A centred empty state: 24 pt `faint` icon, `FONT_HEADING` semibold
/// title, one `dim` line of body and at most one action button (a
/// `ui::button`, Primary for the first-run step, otherwise Secondary).
pub fn empty_state(
    icon: &'static str,
    title: impl Into<SharedString>,
    body: Option<SharedString>,
    action: Option<AnyElement>,
) -> Div {
    div()
        .size_full()
        .min_h_0()
        .p(px(style::S7))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(style::S3))
        .child(
            svg()
                .path(icon)
                .size(px(style::ICON_LARGE))
                .flex_none()
                .mb(px(style::S2))
                .text_color(style::faint()),
        )
        .child(
            div()
                .text_size(px(style::FONT_HEADING))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(style::text())
                .child(title.into()),
        )
        .when_some(body, |state, body| {
            state.child(
                div()
                    .max_w(px(420.))
                    .text_sm()
                    .text_color(style::dim())
                    .text_center()
                    .child(body),
            )
        })
        .when_some(action, |state, action| {
            state.child(div().mt(px(style::S4)).child(action))
        })
}

/// A small info icon whose tooltip carries a caveat ("Readings may change
/// during collection"). The text is also its AX label, so the caveat stays
/// available without hover.
pub fn info_hint(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Stateful<Div> {
    let text = text.into();
    div()
        .id(id)
        .role(Role::Image)
        .aria_label(text.clone())
        .flex_none()
        .size(px(style::CONTROL - 6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(style::R_SM))
        .text_color(style::dim())
        .hover(|s| s.text_color(style::text()))
        .child(
            svg()
                .path("icons/info.svg")
                .size(px(style::ICON))
                .text_color(style::dim()),
        )
        .tooltip(tooltip(text))
        .tooltip_show_delay(tooltip_delay())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_track_fits_the_widest_text_within_bounds() {
        // A short id column stays near the minimum.
        let id = fit_track("id", ["1", "22", "333"], true, 56., 320.);
        assert_eq!(id, 56.);
        // A long header sets the width when values are short.
        let header = fit_track("subscription_id", ["4"], true, 56., 320.);
        assert!(header > 56. && header < 140., "{header}");
        // Long values are capped.
        let long = "x".repeat(400);
        assert_eq!(fit_track("note", [long.as_str()], false, 56., 320.), 320.);
        // Wider text never yields a narrower column.
        let a = fit_track("c", ["abcd"], false, 0., 1000.);
        let b = fit_track("c", ["abcdefgh"], false, 0., 1000.);
        assert!(b > a);
        // A max below the min still returns the min.
        assert_eq!(fit_track("c", ["abcdefgh"], false, 80., 40.), 80.);
    }

    #[test]
    fn fit_track_counts_characters_not_bytes() {
        let ascii = fit_track("", ["abcd"], false, 0., 1000.);
        let wide = fit_track("", ["äöüß"], false, 0., 1000.);
        assert_eq!(ascii, wide);
    }

    #[test]
    fn column_builders_set_alignment_and_family() {
        let column = TableColumn::fixed("Duration", 72.).end().mono();
        assert_eq!(column.track, Track::Fixed(72.));
        assert!(column.end && column.mono);
        let query = TableColumn::flex("Query");
        assert_eq!(query.track, Track::Flex);
        assert!(!query.end && !query.mono);
    }
}

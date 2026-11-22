//! Visual tokens: a compact graphite instrument panel. UI text is 12 pt sans,
//! data is monospace, and one loud signal (the environment colour) frames the
//! window. Views take colours, sizes, spacing and radii from here instead of
//! literals.
// The token set is a contract (DESIGN.md §2): views adopt tokens one by one,
// so a token without a caller yet is not dead design. Remove this allow once
// the round-1 view migration has landed and lint shows nothing unused.
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::{Rgba, rgb, rgba};

/// Root rem; `text_sm` (0.875 rem) lands on 12 pt (`FONT`). `text_xs` lands
/// near 10.3 pt, so set metadata with `text_size(px(FONT_SMALL))` instead.
pub const REM: f32 = 12. / 0.875;

// Type scale (DESIGN.md §2). One family for chrome; `MONO` only for data.
/// Body, labels, buttons, tree rows, menus.
pub const FONT: f32 = 12.;
/// Metadata, captions, footers, table headers.
pub const FONT_SMALL: f32 = 11.;
/// Grid cells, SQL and results (monospace).
pub const FONT_DATA: f32 = 12.;
/// Section headings in pages and tool documents, set semibold, sentence case.
pub const FONT_HEADING: f32 = 13.;
/// Page titles (forms, setup, managed servers), set semibold.
pub const FONT_TITLE: f32 = 17.;
pub const ICON: f32 = 12.;
/// Icon of an `ui::empty_state`.
pub const ICON_LARGE: f32 = 24.;

// Spacing scale. Views use these instead of raw `px(n)` gaps and paddings.
pub const S1: f32 = 2.;
pub const S2: f32 = 4.;
pub const S3: f32 = 6.;
pub const S4: f32 = 8.;
pub const S5: f32 = 12.;
pub const S6: f32 = 16.;
pub const S7: f32 = 24.;

// Heights.
/// Sidebar connections, tree, menus, palette, lists, console.
pub const ROW: f32 = 24.;
/// Data grids, result grids, structure and data tables.
pub const GRID_ROW: f32 = 22.;
/// Every toolbar button, chip, select, segment and toolbar field.
pub const CONTROL: f32 = 24.;
/// Form fields and page buttons.
pub const FIELD: f32 = 28.;
/// Document toolbar row.
pub const TOOLBAR: f32 = 34.;
/// Document footer status line.
pub const FOOTER: f32 = 26.;
/// Window status bar.
pub const STATUS: f32 = 26.;
/// Title row and tab bar.
pub const BAR: f32 = 38.;
pub const SIDEBAR: f32 = 248.;
/// Space the macOS traffic lights occupy at the left of the first row.
pub const TRAFFIC_LIGHTS: f32 = 78.;

// Radii.
/// Rows, tool buttons, chips, badges, checkboxes.
pub const R_SM: f32 = 4.;
/// Fields, buttons, cards, banners.
pub const R_MD: f32 = 6.;
/// Popovers, menus, palette, dialogs.
pub const R_LG: f32 = 10.;

// Insets.
/// Horizontal inset of document toolbars and footers.
pub const INSET_BAR: f32 = S4;
/// Inset of tool and page content.
pub const INSET_CONTENT: f32 = S6;
/// Widest single column of a tool form.
pub const TOOL_COLUMN: f32 = 720.;
/// Label column of `ui::property_list`.
pub const PROPERTY_LABEL: f32 = 120.;
/// Data font: the editor's buffer font, so grid cells and SQL line up.
pub const MONO: &str = ".ZedMono";

/// Motion. Short and quiet: a page settles in, an error nudges, a tooltip
/// fades. Nothing slides across the window. GPUI skips every animation when
/// the system asks for reduced motion.
pub const APPEAR_MS: u64 = 160;
/// How far (px) a new page rises while it fades in.
pub const APPEAR_RISE: f32 = 4.;
pub const SHAKE_MS: u64 = 320;
/// Peak horizontal offset (px) of an error shake.
pub const SHAKE_PX: f32 = 3.;
pub const TOOLTIP_MS: u64 = 120;
pub const TOOLTIP_DELAY_MS: u64 = 450;
/// Sidebar open/close spring: critically damped, settles in about 0.3 s.
pub const SIDEBAR_SPRING: (f32, f32, f32) = (420., 41., 1.);

pub fn bg() -> Rgba {
    rgb(0x0c0d0f)
}
pub fn panel() -> Rgba {
    rgb(0x111316)
}
pub fn raised() -> Rgba {
    rgb(0x171a1e)
}
pub fn hover() -> Rgba {
    rgb(0x1d2126)
}
/// A control held down: darker than `raised`, read as pushed in.
pub fn pressed() -> Rgba {
    rgb(0x0f1114)
}
pub fn select() -> Rgba {
    rgb(0x22324a)
}
pub fn line() -> Rgba {
    rgb(0x24282e)
}
pub fn line_soft() -> Rgba {
    rgb(0x1b1e22)
}
pub fn text() -> Rgba {
    rgb(0xcdd2d9)
}
/// Every informative secondary text: labels, metadata, counts, column
/// types, footers. At least 4.5:1 on `bg`, `panel` and `select`.
pub fn dim() -> Rgba {
    rgb(0x949ca6)
}
/// Disabled text, placeholders and decorative glyphs only. Never for
/// anything a reader needs.
pub fn faint() -> Rgba {
    rgb(0x6e7682)
}
pub fn ok() -> Rgba {
    rgb(0x3fb950)
}
pub fn warn() -> Rgba {
    rgb(0xd29922)
}
pub fn bad() -> Rgba {
    rgb(0xf85149)
}
/// Keyboard focus ring, the selected-cell outline and the solid primary
/// button.
pub fn accent() -> Rgba {
    rgb(0x6aa6ff)
}
/// Focus ring drawn outside a control: `accent` at 60 %.
pub fn focus_ring() -> Rgba {
    rgba(0x6aa6ff99)
}
/// Solid primary button fill on hover: a step brighter than `accent`.
pub fn accent_hover() -> Rgba {
    rgb(0x82b4ff)
}
/// Text on a solid `accent` or `bad` fill.
pub fn on_accent() -> Rgba {
    bg()
}
/// Primary action fill and border: the accent, kept quiet.
pub fn primary_fill() -> Rgba {
    rgba(0x6aa6ff26)
}
pub fn primary_line() -> Rgba {
    rgba(0x6aa6ff73)
}
pub fn primary_text() -> Rgba {
    rgb(0xd6e6ff)
}
/// Error surfaces: field borders, banners and danger buttons.
pub fn bad_fill() -> Rgba {
    rgba(0xf8514924)
}
pub fn bad_line() -> Rgba {
    rgba(0xf8514973)
}
pub fn bad_text() -> Rgba {
    rgb(0xffb3ad)
}
pub fn ok_fill() -> Rgba {
    rgba(0x3fb9501f)
}
/// Caution surfaces: confirmations and policy notes that need a second look.
pub fn warn_fill() -> Rgba {
    rgba(0xd299221c)
}
/// Staged grid edits: edited cells and inserted rows, `warn` at 14 %.
pub fn edited_fill() -> Rgba {
    rgba(0xd2992224)
}
/// Rows staged for deletion, `bad` at 10 %.
pub fn deleted_fill() -> Rgba {
    rgba(0xf851491a)
}
/// Row hover in data grids, between `bg` and `panel`.
pub fn row_hover() -> Rgba {
    rgb(0x14171b)
}
/// Data colours: numbers and booleans in grids, low saturation and close to
/// `text` so identifiers, not numerals, lead the eye.
pub fn number() -> Rgba {
    rgb(0xb7c6da)
}
pub fn boolean() -> Rgba {
    rgb(0xc9bddd)
}

/// Object-kind colours in sidebar trees. Kinds stay apart at a glance while
/// containers and notes keep the quiet text steps.
pub fn kind_table() -> Rgba {
    accent()
}
pub fn kind_view() -> Rgba {
    rgb(0xb392f0)
}
pub fn kind_materialized() -> Rgba {
    rgb(0xd2a8ff)
}
/// Types, domains, dictionaries, foreign tables and triggers.
pub fn kind_type() -> Rgba {
    rgb(0x79c0ff)
}
/// Functions, procedures and aggregates.
pub fn kind_routine() -> Rgba {
    rgb(0x56d4dd)
}

/// What a sidebar tree row shows, independent of engine.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreeKind {
    Database,
    Schema,
    /// A kind group (`Tables`, `Views`).
    Group,
    /// A folder-style group (MySQL).
    Folder,
    ShowMore,
    Warning,
    Info,
    Table,
    View,
    MaterializedView,
    ForeignTable,
    Routine,
    Sequence,
    Type,
    Dictionary,
    Extension,
    Event,
    Trigger,
    Other,
}

/// Icon and colour for a tree row of `kind`.
pub fn kind_icon(kind: TreeKind) -> (&'static str, Rgba) {
    match kind {
        TreeKind::Database => ("icons/database_zap.svg", dim()),
        TreeKind::Schema | TreeKind::Folder => ("icons/folder.svg", dim()),
        TreeKind::Group => ("icons/list_tree.svg", dim()),
        TreeKind::ShowMore => ("icons/plus.svg", faint()),
        TreeKind::Warning => ("icons/warning.svg", warn()),
        TreeKind::Info => ("icons/info.svg", faint()),
        TreeKind::Table => ("icons/table.svg", kind_table()),
        TreeKind::View => ("icons/eye.svg", kind_view()),
        TreeKind::MaterializedView => ("icons/table.svg", kind_materialized()),
        TreeKind::ForeignTable | TreeKind::Trigger => ("icons/link.svg", kind_type()),
        TreeKind::Routine => ("icons/code.svg", kind_routine()),
        TreeKind::Sequence => ("icons/hash.svg", warn()),
        TreeKind::Type | TreeKind::Dictionary => ("icons/box.svg", kind_type()),
        TreeKind::Extension => ("icons/box_open.svg", dim()),
        TreeKind::Event => ("icons/play_outlined.svg", warn()),
        TreeKind::Other => ("icons/circle.svg", dim()),
    }
}

/// Environment signal colour. `None` (no connection) is neutral.
pub fn env(environment: Option<DevelopmentEnvironment>) -> u32 {
    match environment {
        Some(DevelopmentEnvironment::Development) => 0x3fb950,
        Some(DevelopmentEnvironment::Test) => 0x8a929c,
        Some(DevelopmentEnvironment::Staging) => 0xd29922,
        Some(DevelopmentEnvironment::Production) => 0xf85149,
        None => 0x6e7682,
    }
}
/// `alpha` is 0–255, appended to an `env` colour.
pub fn with_alpha(color: u32, alpha: u8) -> Rgba {
    rgba((color << 8) | u32::from(alpha))
}
/// Short environment name. Kept for existing element ids and AX names; new
/// visible text uses `env_short` (tags) or `env_name` (everything else).
pub fn env_label(environment: DevelopmentEnvironment) -> &'static str {
    match environment {
        DevelopmentEnvironment::Development => "Dev",
        DevelopmentEnvironment::Test => "Test",
        DevelopmentEnvironment::Staging => "Stage",
        DevelopmentEnvironment::Production => "Prod",
    }
}
/// Uppercase tag text for `ui::env_tag`: DEV, TEST, STAGE, PROD.
pub fn env_short(environment: DevelopmentEnvironment) -> &'static str {
    match environment {
        DevelopmentEnvironment::Development => "DEV",
        DevelopmentEnvironment::Test => "TEST",
        DevelopmentEnvironment::Staging => "STAGE",
        DevelopmentEnvironment::Production => "PROD",
    }
}
/// Full environment name for visible text outside tags.
pub fn env_name(environment: DevelopmentEnvironment) -> &'static str {
    match environment {
        DevelopmentEnvironment::Development => "Development",
        DevelopmentEnvironment::Test => "Test",
        DevelopmentEnvironment::Staging => "Staging",
        DevelopmentEnvironment::Production => "Production",
    }
}
/// The ~4 % environment wash for chrome surfaces only (sidebar, tab bar,
/// status bar). Documents, grids, editors and dialogs stay neutral.
pub fn env_wash(environment: Option<DevelopmentEnvironment>) -> Rgba {
    with_alpha(env(environment), 0x0a)
}

/// WCAG relative luminance of an opaque colour.
#[cfg(test)]
pub fn luminance(color: Rgba) -> f32 {
    fn channel(c: f32) -> f32 {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
}
/// WCAG contrast ratio between two opaque colours (1.0 to 21.0).
#[cfg(test)]
pub fn contrast(a: Rgba, b: Rgba) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

pub const ENVIRONMENTS: [DevelopmentEnvironment; 4] = [
    DevelopmentEnvironment::Development,
    DevelopmentEnvironment::Test,
    DevelopmentEnvironment::Staging,
    DevelopmentEnvironment::Production,
];

/// Short engine badge and its colour.
pub fn engine_badge(engine: &str) -> (&'static str, u32) {
    match engine {
        "PostgreSQL" => ("PG", 0x6c9bd2),
        "MySQL" => ("MY", 0xe6a23c),
        "ClickHouse" => ("CH", 0xf4d03f),
        "Redis" => ("RD", 0xe5534b),
        "SQLite" => ("SQ", 0x8bb8a8),
        _ => ("DB", 0x8a929c),
    }
}

/// Embedded logo for an engine, shown in place of its badge.
pub fn engine_logo(engine: &str) -> Option<&'static str> {
    match engine {
        "PostgreSQL" => Some("logos/postgres.svg"),
        "MySQL" => Some("logos/mysql.svg"),
        "ClickHouse" => Some("logos/clickhouse.svg"),
        "Redis" => Some("logos/redis.svg"),
        "SQLite" => Some("logos/sqlite-mark.svg"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_colours_are_distinct_and_alpha_composes() {
        let colours: Vec<u32> = ENVIRONMENTS.iter().map(|e| env(Some(*e))).collect();
        for (i, a) in colours.iter().enumerate() {
            assert!(colours[i + 1..].iter().all(|b| a != b));
        }
        assert_eq!(with_alpha(0x3fb950, 0x0d), rgba(0x3fb9500d));
        assert_eq!(engine_badge("Redis").0, "RD");
        assert_eq!(engine_badge("Unknown").0, "DB");
    }

    #[test]
    fn informative_text_meets_contrast_on_every_resting_surface() {
        for surface in [bg(), panel(), raised(), select()] {
            assert!(contrast(text(), surface) >= 7., "text on {surface:?}");
            assert!(contrast(dim(), surface) >= 4.5, "dim on {surface:?}");
        }
        // Faint is for disabled text and placeholders, but stays legible.
        assert!(contrast(faint(), panel()) >= 3.5);
        assert!(contrast(dim(), faint()) > 1.2);
        // Text on the solid primary button.
        assert!(contrast(on_accent(), accent()) >= 4.5);
    }

    #[test]
    fn contrast_is_symmetric_and_bounded() {
        let white = rgb(0xffffff);
        let black = rgb(0x000000);
        assert!((contrast(white, black) - 21.).abs() < 0.01);
        assert!((contrast(black, white) - 21.).abs() < 0.01);
        assert!((contrast(text(), text()) - 1.).abs() < 1e-4);
    }

    #[test]
    fn type_scale_and_heights_follow_the_contract() {
        assert!((REM * 0.875 - FONT).abs() < 1e-3);
        const { assert!(FONT_SMALL < FONT && FONT < FONT_HEADING && FONT_HEADING < FONT_TITLE) };
        const { assert!(CONTROL < FIELD && GRID_ROW <= ROW) };
        const { assert!(R_SM < R_MD && R_MD < R_LG) };
        assert!([S1, S2, S3, S4, S5, S6, S7].windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn environment_names_are_short_in_tags_and_full_elsewhere() {
        let short: Vec<_> = ENVIRONMENTS.iter().map(|e| env_short(*e)).collect();
        assert_eq!(short, ["DEV", "TEST", "STAGE", "PROD"]);
        assert_eq!(env_name(DevelopmentEnvironment::Staging), "Staging");
        // Existing AX names and ids keep the old labels.
        assert_eq!(env_label(DevelopmentEnvironment::Staging), "Stage");
    }

    #[test]
    fn staged_tints_derive_from_their_status_colours() {
        assert_eq!(edited_fill(), with_alpha(0xd29922, 0x24));
        assert_eq!(deleted_fill(), with_alpha(0xf85149, 0x1a));
    }

    #[test]
    fn tree_kinds_keep_containers_quiet_and_objects_distinct() {
        for container in [TreeKind::Database, TreeKind::Schema, TreeKind::Group] {
            assert_eq!(kind_icon(container).1, dim());
        }
        let objects = [
            TreeKind::Table,
            TreeKind::View,
            TreeKind::MaterializedView,
            TreeKind::Routine,
            TreeKind::Type,
        ];
        for (i, a) in objects.iter().enumerate() {
            assert!(
                objects[i + 1..]
                    .iter()
                    .all(|b| kind_icon(*a).1 != kind_icon(*b).1)
            );
        }
    }
}

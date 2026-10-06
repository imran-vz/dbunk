//! Plan 031 visual tokens: a compact graphite instrument panel at 11 px, data
//! in monospace, and one loud signal — the environment colour that frames the
//! window. Views take colours and sizes from here instead of literals.
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::{Rgba, rgb, rgba};

/// Root rem; `text_sm` (0.875 rem) lands on 11 px and `text_xs` near 9.5 px.
pub const REM: f32 = 12.6;
pub const FONT: f32 = 11.;
pub const FONT_SMALL: f32 = 10.;
/// Page titles (forms, setup, managed servers), set semibold.
pub const FONT_TITLE: f32 = 15.;
pub const ROW: f32 = 20.;
pub const BAR: f32 = 34.;
pub const STATUS: f32 = 22.;
pub const ICON: f32 = 11.;
pub const SIDEBAR: f32 = 248.;
/// Space the macOS traffic lights occupy at the left of the first row.
pub const TRAFFIC_LIGHTS: f32 = 78.;
/// Document toolbar row, footer status line and tool button heights.
pub const TOOLBAR: f32 = 28.;
pub const FOOTER: f32 = 24.;
pub const TOOL: f32 = 20.;
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
pub fn dim() -> Rgba {
    rgb(0x8a929c)
}
pub fn faint() -> Rgba {
    rgb(0x5b626c)
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
/// Keyboard focus ring and the selected-cell outline.
pub fn accent() -> Rgba {
    rgb(0x6aa6ff)
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
/// Row hover in data grids, between `bg` and `panel`.
pub fn row_hover() -> Rgba {
    rgb(0x14171b)
}
/// Data colours: numbers and booleans in grids.
pub fn number() -> Rgba {
    rgb(0x79c0ff)
}
pub fn boolean() -> Rgba {
    rgb(0xd2a8ff)
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
        None => 0x5b626c,
    }
}
/// `alpha` is 0–255, appended to an `env` colour.
pub fn with_alpha(color: u32, alpha: u8) -> Rgba {
    rgba((color << 8) | u32::from(alpha))
}
pub fn env_label(environment: DevelopmentEnvironment) -> &'static str {
    match environment {
        DevelopmentEnvironment::Development => "Dev",
        DevelopmentEnvironment::Test => "Test",
        DevelopmentEnvironment::Staging => "Stage",
        DevelopmentEnvironment::Production => "Prod",
    }
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
            assert!(objects[i + 1..].iter().all(|b| kind_icon(*a).1 != kind_icon(*b).1));
        }
    }
}

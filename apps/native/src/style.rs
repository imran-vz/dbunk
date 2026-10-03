//! Plan 031 visual tokens: a compact graphite instrument panel at 11 px, data
//! in monospace, and one loud signal — the environment colour that frames the
//! window. Views take colours and sizes from here instead of literals.
use dbunk_lib::backend::DevelopmentEnvironment;
use gpui::{Rgba, rgb, rgba};

/// Root rem; `text_sm` (0.875 rem) lands on 11 px and `text_xs` near 9.5 px.
pub const REM: f32 = 12.6;
pub const FONT: f32 = 11.;
pub const FONT_SMALL: f32 = 10.;
pub const ROW: f32 = 20.;
pub const BAR: f32 = 34.;
pub const STATUS: f32 = 22.;
pub const ICON: f32 = 11.;
pub const SIDEBAR: f32 = 248.;
/// Space the macOS traffic lights occupy at the left of the first row.
pub const TRAFFIC_LIGHTS: f32 = 78.;

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
}

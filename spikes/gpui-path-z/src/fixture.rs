//! The Plan 024 grid fixtures, cell for cell what
//! `tools/measure/fixtures/postgres.sql` returns from PostgreSQL.

use gpui::SharedString;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fixture {
    Wide,
    Large,
    Many,
}

impl Fixture {
    /// The fixture a statement selects from, if any.
    pub fn from_sql(sql: &str) -> Option<Self> {
        let sql = sql.to_ascii_lowercase();
        if sql.contains("fixture_wide") {
            Some(Self::Wide)
        } else if sql.contains("fixture_large") {
            Some(Self::Large)
        } else if sql.contains("fixture_many") {
            Some(Self::Many)
        } else {
            None
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::from_sql(&format!("fixture_{name}"))
    }

    pub fn columns(self) -> Vec<SharedString> {
        match self {
            Self::Wide => (0..100).map(|k| format!("c{k:03}").into()).collect(),
            Self::Large => (0..8).map(|k| format!("t{k}").into()).collect(),
            Self::Many => [
                "id", "doubled", "label", "padded", "bucket", "note", "day", "body",
            ]
            .into_iter()
            .map(Into::into)
            .collect(),
        }
    }

    pub fn row_count(self) -> usize {
        match self {
            Self::Wide | Self::Many => 10_000,
            Self::Large => 400,
        }
    }

    /// Row `g`, counted from 1 as `generate_series` does.
    pub fn row(self, g: usize) -> Vec<String> {
        match self {
            Self::Wide => (0..100).map(|k| (g + k).to_string()).collect(),
            Self::Large => (0..8)
                .map(|k| format!("{:x>16}", g * 31 + k).repeat(512))
                .collect(),
            Self::Many => vec![
                g.to_string(),
                (g * 2).to_string(),
                format!("row-{g}"),
                format!("{g:0>10}"),
                (g % 97).to_string(),
                format!("value {}", g * 7),
                "2026-10-02".into(),
                format!("lorem ipsum dolor sit amet {g}"),
            ],
        }
    }
}

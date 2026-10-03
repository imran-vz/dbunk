//! SQL for Zed's editor without a language server: a Tree-sitter grammar for
//! highlighting. No language server or schema service is started.

use std::sync::Arc;

use gpui::App;
use language::{Language, LanguageConfig, LanguageMatcher};
use theme::ActiveTheme;

/// Exact templates from the Tauri query toolbar at parity baseline 102568b.
#[derive(Clone, Copy)]
pub enum Snippet {
    TopRows,
    GroupedCount,
    RecentRows,
}
impl Snippet {
    pub fn sql(self) -> &'static str {
        match self {
            Self::TopRows => "select *\nfrom public.table_name\nlimit 100;",
            Self::GroupedCount => {
                "select column_name, count(*)\nfrom public.table_name\ngroup by column_name\norder by count(*) desc;"
            }
            Self::RecentRows => {
                "select *\nfrom public.table_name\nwhere created_at >= now() - interval '7 days'\norder by created_at desc;"
            }
        }
    }
}

pub fn language(cx: &App) -> anyhow::Result<Arc<Language>> {
    let language = Language::new(
        LanguageConfig {
            name: "SQL".into(),
            matcher: LanguageMatcher {
                path_suffixes: vec!["sql".into()],
                ..Default::default()
            }
            .into(),
            line_comments: vec!["-- ".into()],
            ..Default::default()
        },
        Some(tree_sitter_sequel::LANGUAGE.into()),
    )
    .with_highlights_query(tree_sitter_sequel::HIGHLIGHTS_QUERY)?;
    // The language registry normally does this when a theme loads.
    language.set_theme(cx.theme().syntax());
    Ok(Arc::new(language))
}

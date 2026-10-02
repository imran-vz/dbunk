//! SQL for Zed's editor without a language server: a Tree-sitter grammar for
//! highlighting. No language server or schema service is started.

use std::sync::Arc;

use gpui::App;
use language::{Language, LanguageConfig, LanguageMatcher};
use theme::ActiveTheme;

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

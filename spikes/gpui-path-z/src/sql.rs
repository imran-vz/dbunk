//! SQL for Zed's editor without a language server: a Tree-sitter grammar for
//! highlighting and a `CompletionProvider` that answers from a schema the
//! application supplies.

use std::sync::Arc;

use editor::{CompletionContext, CompletionProvider, Editor};
use gpui::{App, Context, Entity, Task, Window};
use language::{Buffer, CodeLabel, Language, LanguageConfig, LanguageMatcher};
use project::{Completion, CompletionDisplayOptions, CompletionResponse, CompletionSource};
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

/// JSON for the cell editor probe.
pub fn json_language(cx: &App) -> anyhow::Result<Arc<Language>> {
    let language = Language::new(
        LanguageConfig {
            name: "JSON".into(),
            ..Default::default()
        },
        Some(tree_sitter_json::LANGUAGE.into()),
    )
    .with_highlights_query(tree_sitter_json::HIGHLIGHTS_QUERY)?;
    language.set_theme(cx.theme().syntax());
    Ok(Arc::new(language))
}

/// Completes schema names. In dbunk this would read the schema cache of the
/// tab's connection; here it is a fixed list.
pub struct SchemaCompletions {
    pub names: Vec<&'static str>,
}

impl CompletionProvider for SchemaCompletions {
    fn completions(
        &self,
        buffer: &Entity<Buffer>,
        buffer_position: language::Anchor,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<Editor>,
    ) -> Task<anyhow::Result<Vec<CompletionResponse>>> {
        let buffer = buffer.read(cx);
        let word = buffer
            .reversed_chars_at(buffer_position)
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .count();
        let end = text::ToOffset::to_offset(&buffer_position, buffer);
        let replace_range = buffer.anchor_before(end.saturating_sub(word))..buffer_position;
        Task::ready(Ok(vec![CompletionResponse {
            completions: self
                .names
                .iter()
                .map(|name| Completion {
                    replace_range: replace_range.clone(),
                    new_text: name.to_string(),
                    label: CodeLabel::plain(name.to_string(), None),
                    documentation: None,
                    source: CompletionSource::Custom,
                    icon_path: None,
                    icon_color: None,
                    match_start: None,
                    snippet_deduplication_key: None,
                    insert_text_mode: None,
                    confirm: None,
                    group: None,
                })
                .collect(),
            display_options: CompletionDisplayOptions::default(),
            is_incomplete: false,
        }]))
    }

    fn is_completion_trigger(
        &self,
        buffer: &Entity<Buffer>,
        position: language::Anchor,
        text: &str,
        _trigger_in_words: bool,
        cx: &mut Context<Editor>,
    ) -> bool {
        // No menu inside a line comment, as in the current editor.
        let buffer = buffer.read(cx);
        let point = text::ToPoint::to_point(&position, buffer);
        let line_start = text::Point::new(point.row, 0);
        let prefix: String = buffer.text_for_range(line_start..point).collect();
        if prefix.trim_start().starts_with("--") {
            return false;
        }
        text.chars()
            .last()
            .is_some_and(|last| last.is_ascii_alphanumeric() || last == '_' || last == '.')
    }
}

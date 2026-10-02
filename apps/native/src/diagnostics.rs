//! Local query diagnostics for the pinned editor. No language server is started.
//!
//! PostgreSQL positions count Unicode scalar values from one. The retained source
//! range converts them to editor bytes without searching for repeated SQL text.
use std::{ops::Range, rc::Rc, sync::Arc};

use collections::{HashMap, HashSet};
use editor::{
    DiagnosticRenderer, Editor, EditorSnapshot, GotoDefinitionKind, RenameTarget,
    SemanticsProvider, display_map::BlockProperties,
};
use futures_util::future::Shared;
use gpui::{App, AppContext, Context, Entity, SharedString, Task, WeakEntity, Window};
use language::{
    Buffer, BufferRow, Diagnostic, DiagnosticEntry, DiagnosticEntryRef, DiagnosticMessage,
    DiagnosticSet, LanguageRegistry, Point,
};
use lsp::LanguageServerId;
use project::{
    DocumentHighlight, InlayHint, InvalidationStrategy, LocationLink, ProjectTransaction,
    lsp_store::{BufferSemanticTokens, CacheInlayHints},
};
use text::{Anchor, BufferId};
use unicode_segmentation::UnicodeSegmentation;

// This local buffer has no LSP. A stable source id lets updates replace the old error.
const QUERY_DIAGNOSTIC: LanguageServerId = LanguageServerId(0);

pub struct ExecutedSql {
    source: String,
    range: Range<usize>,
}

impl ExecutedSql {
    pub fn new(source: String, range: Range<usize>) -> Option<Self> {
        source.get(range.clone())?;
        if !source.is_grapheme_boundary(range.start) || !source.is_grapheme_boundary(range.end) {
            return None;
        }
        Some(Self { source, range })
    }

    pub fn range_for_position(
        &self,
        position: Option<u32>,
        current_text: &str,
    ) -> Option<Range<usize>> {
        if current_text != self.source {
            return None;
        }
        let index = usize::try_from(position?.checked_sub(1)?).ok()?;
        let sql = &self.source[self.range.clone()];
        let offset = sql
            .char_indices()
            .map(|(byte, _)| byte)
            .chain(std::iter::once(sql.len()))
            .nth(index)?;
        let relative = token_range(sql, offset);
        Some(self.range.start + relative.start..self.range.start + relative.end)
    }
}

fn token_range(sql: &str, offset: usize) -> Range<usize> {
    if offset == sql.len() {
        return offset..offset;
    }
    // Only a grammar leaf is a known token. Recovery nodes, whitespace, and
    // absent grammar information fall back to the reported grapheme, not a guess.
    let mut parser = language::Parser::new();
    if parser
        .set_language(&tree_sitter_sequel::LANGUAGE.into())
        .is_ok()
        && let Some(tree) = parser.parse(sql, None)
        && let Some(node) = tree.root_node().descendant_for_byte_range(offset, offset)
        && node.child_count() == 0
        && !node.is_error()
        && !node.is_missing()
        && node.start_byte() <= offset
        && offset < node.end_byte()
    {
        let range = node.byte_range();
        // A grammar can split combining sequences; preserve the visible character.
        if sql.is_grapheme_boundary(range.start) && sql.is_grapheme_boundary(range.end) {
            return range;
        }
    }
    sql.grapheme_indices(true)
        .find_map(|(start, grapheme)| {
            let end = start + grapheme.len();
            (start <= offset && offset < end).then_some(start..end)
        })
        .unwrap_or(offset..offset)
}

trait GraphemeBoundary {
    fn is_grapheme_boundary(&self, offset: usize) -> bool;
}

impl GraphemeBoundary for str {
    fn is_grapheme_boundary(&self, offset: usize) -> bool {
        offset == self.len()
            || self
                .grapheme_indices(true)
                .any(|(start, _)| start == offset)
    }
}

pub fn install(editor: &Entity<Editor>, cx: &mut App) {
    editor::set_diagnostic_renderer(QueryDiagnosticRenderer, cx);
    editor.update(cx, |editor, _| {
        editor.set_semantics_provider(Some(Rc::new(LocalSemantics)));
        // Keep the approved compact hover and result-boundary error strip as the
        // two presentations; do not duplicate messages at the end of SQL lines.
        editor.disable_inline_diagnostics();
    });
}

pub fn set_error(
    buffer: &Entity<Buffer>,
    range: Range<usize>,
    message: &str,
    code: Option<&str>,
    cx: &mut App,
) {
    buffer.update(cx, |buffer, cx| {
        let snapshot = buffer.snapshot();
        let entry = DiagnosticEntry::new(
            snapshot.anchor_before(range.start)..snapshot.anchor_after(range.end),
            Diagnostic {
                source: Some("PostgreSQL".into()),
                message: DiagnosticMessage::plain(message.to_owned()),
                code: code.map(|code| lsp::NumberOrString::String(code.to_owned())),
                is_primary: true,
                ..Default::default()
            },
        );
        let set = DiagnosticSet::from_sorted_entries([entry], &snapshot);
        buffer.update_diagnostics(QUERY_DIAGNOSTIC, set, cx);
    });
}

pub fn clear(editor: &Entity<Editor>, buffer: &Entity<Buffer>, cx: &mut App) {
    editor.update(cx, editor::hover_popover::hide_hover);
    buffer.update(cx, |buffer, cx| {
        let empty = DiagnosticSet::from_sorted_entries([], &buffer.snapshot());
        buffer.update_diagnostics(QUERY_DIAGNOSTIC, empty, cx);
    });
}

struct QueryDiagnosticRenderer;

impl DiagnosticRenderer for QueryDiagnosticRenderer {
    fn render_group(
        &self,
        _: Vec<DiagnosticEntryRef<'_, Point>>,
        _: BufferId,
        _: EditorSnapshot,
        _: WeakEntity<Editor>,
        _: Option<Arc<LanguageRegistry>>,
        _: &mut App,
    ) -> Vec<BlockProperties<editor::Anchor>> {
        Vec::new()
    }

    fn render_hover(
        &self,
        group: Vec<DiagnosticEntryRef<'_, Point>>,
        _: Range<Point>,
        _: BufferId,
        _: Option<Arc<LanguageRegistry>>,
        cx: &mut App,
    ) -> Option<Entity<markdown::Markdown>> {
        let diagnostic = group
            .iter()
            .find(|entry| entry.diagnostic.is_primary)?
            .diagnostic;
        let code = match &diagnostic.code {
            Some(lsp::NumberOrString::String(code)) => format!(" · {code}"),
            Some(lsp::NumberOrString::Number(code)) => format!(" · {code}"),
            None => String::new(),
        };
        let content = format!("{}\nPostgreSQL{code}", diagnostic.message.as_str());
        // Treat database messages as text, never as rich Markdown or images.
        Some(cx.new(|cx| markdown::Markdown::new_text(content.into(), cx)))
    }

    fn open_link(&self, _: &mut Editor, _: SharedString, _: &mut Window, _: &mut Context<Editor>) {}
}

/// Zed requires a semantics provider before checking buffer diagnostics. All
/// capabilities remain disabled; the only data comes from our local buffer.
struct LocalSemantics;

impl SemanticsProvider for LocalSemantics {
    fn hover(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        _: &mut App,
    ) -> Option<Task<Option<Vec<project::Hover>>>> {
        None
    }
    fn inline_values(
        &self,
        _: Entity<Buffer>,
        _: Range<Anchor>,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Vec<InlayHint>>>> {
        None
    }
    fn applicable_inlay_chunks(
        &self,
        _: &Entity<Buffer>,
        _: &[Range<Anchor>],
        _: &mut App,
    ) -> Vec<Range<BufferRow>> {
        Vec::new()
    }
    fn invalidate_inlay_hints(&self, _: &HashSet<BufferId>, _: &mut App) {}
    fn inlay_hints(
        &self,
        _: InvalidationStrategy,
        _: Entity<Buffer>,
        _: Vec<Range<Anchor>>,
        _: Option<(clock::Global, HashSet<Range<BufferRow>>)>,
        _: &mut App,
    ) -> Option<HashMap<Range<BufferRow>, Task<anyhow::Result<CacheInlayHints>>>> {
        None
    }
    fn semantic_tokens(
        &self,
        _: Entity<Buffer>,
        _: &mut App,
    ) -> Option<Shared<Task<Result<BufferSemanticTokens, Arc<anyhow::Error>>>>> {
        None
    }
    fn supports_inlay_hints(&self, _: &Entity<Buffer>, _: &mut App) -> bool {
        false
    }
    fn supports_semantic_tokens(&self, _: &Entity<Buffer>, _: &mut App) -> bool {
        false
    }
    fn document_highlights(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Vec<DocumentHighlight>>>> {
        None
    }
    fn definitions(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        _: GotoDefinitionKind,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<Option<Vec<LocationLink>>>>> {
        None
    }
    fn range_for_rename(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        _: &mut App,
    ) -> Task<anyhow::Result<Option<RenameTarget>>> {
        Task::ready(Ok(None))
    }
    fn perform_rename(
        &self,
        _: &Entity<Buffer>,
        _: Anchor,
        _: String,
        _: Option<LanguageServerId>,
        _: &mut App,
    ) -> Option<Task<anyhow::Result<ProjectTransaction>>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(source: &str, selection: Range<usize>, position: u32) -> Range<usize> {
        ExecutedSql::new(source.to_owned(), selection)
            .unwrap()
            .range_for_position(Some(position), source)
            .unwrap()
    }

    #[test]
    fn maps_selected_duplicate_without_searching() {
        let statement = "SELECT * FROM missing_table;";
        let source = format!("{statement}\n{statement}");
        let start = statement.len() + 1;
        let range = map(&source, start..source.len(), 15);
        assert_eq!(range, start + 14..start + 27);
        assert_eq!(&source[range], "missing_table");
    }

    #[test]
    fn unicode_prefix_and_later_script_statement_use_character_positions() {
        let sql = "SELECT '🦀é'; SELECT * FROM missing_table;";
        let start = sql.find("missing_table").unwrap();
        let position = sql[..start].chars().count() as u32 + 1;
        assert_eq!(map(sql, 0..sql.len(), position), start..start + 13);
    }

    #[test]
    fn invalid_missing_and_stale_positions_do_not_mark_sql() {
        let sql = "SELECT 1";
        let executed = ExecutedSql::new(sql.into(), 0..sql.len()).unwrap();
        for position in [None, Some(0), Some(100), Some(u32::MAX)] {
            assert!(executed.range_for_position(position, sql).is_none());
        }
        assert!(executed.range_for_position(Some(1), "SELECT 2").is_none());
        assert!(ExecutedSql::new("🦀".into(), 1..4).is_none());
        // A valid UTF-8 selection may still bisect a visible combining sequence.
        assert!(ExecutedSql::new("é".into(), 1..3).is_none());
        assert!(ExecutedSql::new("é".into(), 0..1).is_none());
    }

    #[test]
    fn end_of_input_is_an_empty_range_at_the_executed_end() {
        let sql = "SELECT '🦀'; trailing text";
        let end = sql.find(';').unwrap() + 1;
        let position = sql[..end].chars().count() as u32 + 1;
        assert_eq!(map(sql, 0..end, position), end..end);
    }

    #[test]
    fn recovery_fallback_keeps_combining_sequence_intact() {
        let sql = "SELECT 'é';";
        let range = map(sql, 0..sql.len(), 10);
        assert!(sql.is_grapheme_boundary(range.start));
        assert!(sql.is_grapheme_boundary(range.end));
        assert!(range.contains(&9));
    }
}

use super::*;
use editor::{CompletionContext, CompletionProvider, Editor};
use gpui::{AppContext, Entity, EntityInputHandler, Task};
use language::{Buffer, CodeLabel};
use project::{Completion, CompletionResponse, CompletionSource};
use text::ToOffset;

type CompletionConfirm =
    std::sync::Arc<dyn Send + Sync + Fn(project::CompletionIntent, &mut Window, &mut App) -> bool>;

pub(super) struct Provider(pub CompletionHandle);
struct Candidate {
    label: String,
    insert: String,
    detail: String,
}
struct Candidates {
    values: Vec<Candidate>,
    bytes: usize,
    limited: bool,
}
impl Candidates {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            bytes: 0,
            limited: false,
        }
    }
    fn push(&mut self, label: String, insert: String, detail: String) {
        let size = label
            .len()
            .saturating_add(insert.len())
            .saturating_add(detail.len());
        if self.values.len() == MENU_ITEMS || size > MENU_TEXT_BYTES.saturating_sub(self.bytes) {
            self.limited = true;
            return;
        }
        if self.values.iter().any(|c| c.insert == insert) {
            return;
        }
        self.bytes += size;
        self.values.push(Candidate {
            label,
            insert,
            detail,
        });
    }
}
impl CompletionProvider for Provider {
    fn completions(
        &self,
        buffer: &Entity<Buffer>,
        position: text::Anchor,
        _trigger: CompletionContext,
        window: &mut Window,
        cx: &mut Context<Editor>,
    ) -> Task<anyhow::Result<Vec<CompletionResponse>>> {
        let handle = self.0.clone();
        let generation = handle.0.borrow().generation;
        let buffer = buffer.clone();
        let version = buffer.read(cx).snapshot().version().clone();
        // Defer until the Editor invocation has released its mutable borrow.
        // Then read actual marked text and current caret before building anything.
        cx.spawn_in(window, async move |editor, cx| {
            editor.update_in(cx, |editor, window, cx| {
                if editor.marked_text_range(window, cx).is_some() || editor.read_only(cx) {
                    return Ok(Vec::new());
                }
                let snapshot = buffer.read(cx).snapshot();
                if snapshot.version() != &version {
                    return Ok(Vec::new());
                }
                let Some((current_buffer, current_position)) = editor
                    .buffer()
                    .read(cx)
                    .text_anchor_for_position(editor.selections.newest_anchor().head(), cx)
                else {
                    return Ok(Vec::new());
                };
                if current_buffer != buffer
                    || current_position.to_offset(&snapshot) != position.to_offset(&snapshot)
                {
                    return Ok(Vec::new());
                }
                let offset = position.to_offset(&snapshot);
                if offset > context::CONTEXT_BYTES {
                    let mut state = handle.0.borrow_mut();
                    state.status =
                        "SQL completion context exceeds 64 KiB; place SQL in a smaller document"
                            .into();
                    let _ = state.wake.try_send(());
                    return Ok(Vec::new());
                }
                let mut state = handle.0.borrow_mut();
                if generation != state.generation || state.composing {
                    return Ok(Vec::new());
                }
                let lease = match Lease::admit(state.budget.clone(), MENU_BYTES) {
                    Ok(lease) => lease,
                    Err(error) => {
                        state.status = error.into();
                        let _ = state.wake.try_send(());
                        return Ok(Vec::new());
                    }
                };
                let before = snapshot.text_for_range(0..offset).collect::<String>();
                // Whole-token replacement only needs a bounded tail. A token
                // reaching the cap refuses, rather than leaving a suffix behind.
                let after = snapshot
                    .text_for_range(offset..snapshot.len())
                    .flat_map(str::chars)
                    .take(1025)
                    .collect::<String>();
                let Some(context) = context::parse(&before, &after) else {
                    return Ok(Vec::new());
                };
                if context.range.end - offset >= after.len() && after.len() >= 1025 {
                    return Ok(Vec::new());
                }
                let candidates = state.candidates(&context);
                if candidates.values.is_empty() {
                    return Ok(Vec::new());
                }
                let range = snapshot.anchor_before(context.range.start)
                    ..snapshot.anchor_after(context.range.end);
                drop(state);
                // GPUI owns the lease. Every cloned Completion retains this
                // entity through its callback, including Zed's menu and accept
                // clones. The callback is a lifetime hook, NOT an acceptance veto.
                let lease = cx.new(|_| lease);
                let confirm: CompletionConfirm = std::sync::Arc::new(move |_, _, _| {
                    let _keep = &lease;
                    false
                });
                let completions = candidates
                    .values
                    .into_iter()
                    .map(|candidate| {
                        let filter_text = candidate.label.clone();
                        let label = if candidate.detail.is_empty() {
                            candidate.label
                        } else {
                            format!("{}  {}", candidate.label, candidate.detail)
                        };
                        Completion {
                            replace_range: range.clone(),
                            new_text: candidate.insert,
                            label: CodeLabel::plain(label, Some(&filter_text)),
                            documentation: None,
                            source: CompletionSource::Custom,
                            icon_path: None,
                            icon_color: None,
                            match_start: None,
                            snippet_deduplication_key: None,
                            insert_text_mode: Some(lsp::InsertTextMode::AS_IS),
                            confirm: Some(confirm.clone()),
                            group: None,
                        }
                    })
                    .collect();
                Ok(vec![CompletionResponse {
                    completions,
                    display_options: Default::default(),
                    is_incomplete: true,
                }])
            })?
        })
    }
    fn is_completion_trigger(
        &self,
        _buffer: &Entity<Buffer>,
        _position: text::Anchor,
        text: &str,
        trigger_in_words: bool,
        _cx: &mut Context<Editor>,
    ) -> bool {
        let state = self.0.0.borrow();
        !state.composing
            && (matches!(text, " " | "." | "\n" | "\"")
                || (trigger_in_words && text.chars().all(|c| c.is_alphanumeric() || c == '_')))
    }
    // Our bounded prefix filter and ordering are deterministic and avoid an
    // independent fuzzy match policy reordering current-schema/column priority.
    fn sort_completions(&self) -> bool {
        false
    }
    fn filter_completions(&self) -> bool {
        false
    }
}
impl State {
    fn candidates(&mut self, context: &context::Context) -> Candidates {
        let mut result = Candidates::new();
        let matches = |name: &str| {
            if context.quoted {
                name.starts_with(&context.prefix)
            } else {
                name.to_lowercase()
                    .starts_with(&context.prefix.to_lowercase())
            }
        };
        let keywords_first = !context.relation_position && context.target.is_none();
        if keywords_first {
            add_keywords(&mut result, context, &matches);
        }
        let mut need = None;
        if self.connected {
            if let Some(cache) = &self.cache {
                // No SQL-session search_path is inferred from the metadata
                // socket. Public is the baseline fallback, not an authority.
                let default = cache
                    .schemas
                    .iter()
                    .find(|s| s.name == "public")
                    .or_else(|| cache.schemas.first());
                if let Some((schema, relation)) = &context.target {
                    let schema = schema
                        .as_deref()
                        .or_else(|| default.map(|s| s.name.as_str()));
                    if let Some(schema) = schema {
                        if let Some(columns) = cache
                            .columns
                            .as_ref()
                            .filter(|c| c.schema == schema && c.relation == *relation)
                        {
                            for column in &columns.columns {
                                if matches(&column.name) {
                                    let detail = format!(
                                        "{}{}",
                                        column.data_type.chars().take(120).collect::<String>(),
                                        if column.is_primary_key {
                                            " primary key"
                                        } else {
                                            ""
                                        }
                                    );
                                    result.push(
                                        column.name.clone(),
                                        context::quote(&column.name),
                                        detail,
                                    );
                                }
                            }
                        } else {
                            need = Some((schema.to_owned(), relation.clone()));
                        }
                    }
                } else if let Some(qualifier) = &context.schema {
                    if let Some(schema) = cache.schemas.iter().find(|s| s.name == *qualifier) {
                        add_relations(&mut result, schema, false, &matches);
                    }
                } else {
                    if context.relation_position
                        && let Some(schema) = default
                    {
                        add_relations(&mut result, schema, false, &matches);
                    }
                    for schema in &cache.schemas {
                        if matches(&schema.name) && schema.name.len() <= 256 {
                            result.push(
                                schema.name.clone(),
                                context::quote(&schema.name),
                                "Schema".into(),
                            );
                        }
                    }
                    for schema in &cache.schemas {
                        if !context.relation_position
                            || Some(schema.name.as_str()) != default.map(|s| s.name.as_str())
                        {
                            add_relations(
                                &mut result,
                                schema,
                                Some(schema.name.as_str()) != default.map(|s| s.name.as_str()),
                                &matches,
                            );
                        }
                    }
                }
            } else {
                self.queue(None);
            }
        }
        if let Some(key) = need {
            self.queue(Some(key));
        }
        if !keywords_first {
            add_keywords(&mut result, context, &matches);
        }
        if result.limited {
            self.status = format!(
                "SQL completion limited to 128 candidates / 32 KiB; type a longer prefix{}{}",
                if self.cache.as_ref().is_some_and(|c| c.truncated) {
                    "; catalog is partial"
                } else {
                    ""
                },
                if self.failed.is_some() {
                    "; metadata unavailable, refresh to retry"
                } else {
                    ""
                }
            );
        }
        let _ = self.wake.try_send(());
        result
    }
}
fn add_keywords(
    result: &mut Candidates,
    context: &context::Context,
    matches: &impl Fn(&str) -> bool,
) {
    if context.schema.is_none() && !context.quoted {
        for keyword in context::KEYWORDS {
            if matches(keyword) {
                result.push((*keyword).into(), (*keyword).into(), "Keyword".into());
            }
        }
    }
}
fn add_relations(
    result: &mut Candidates,
    schema: &Schema,
    qualified: bool,
    matches: &impl Fn(&str) -> bool,
) {
    for relation in &schema.relations {
        if schema.name.len() > 256 || relation.name.len() > 256 {
            result.limited = true;
            continue;
        }
        let label = if qualified {
            format!("{}.{}", schema.name, relation.name)
        } else {
            relation.name.clone()
        };
        if !matches(&label) && !(qualified && matches(&relation.name)) {
            continue;
        }
        let insert = if qualified {
            format!(
                "{}.{}",
                context::quote(&schema.name),
                context::quote(&relation.name)
            )
        } else {
            context::quote(&relation.name)
        };
        result.push(
            label,
            insert,
            format!(
                "{} in {}",
                if relation.view { "View" } else { "Table" },
                schema.name
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_cap_and_exact_insert_are_independent_from_display() {
        let mut candidates = Candidates::new();
        for i in 0..1000 {
            candidates.push(
                format!("name{i}"),
                context::quote(&format!("name{i}")),
                "Table".into(),
            );
        }
        assert_eq!(candidates.values.len(), MENU_ITEMS);
        assert!(candidates.limited);
        assert!(candidates.bytes <= MENU_TEXT_BYTES);
        assert_eq!(candidates.values[0].insert, "\"name0\"");
    }
    #[test]
    fn disconnected_only_keywords_no_metadata_request() {
        let (wake, _) = async_channel::bounded(1);
        let h = CompletionHandle::new(Rc::new(Cell::new(0)), wake);
        let context = context::parse("sel", "").unwrap();
        let candidates = h.0.borrow_mut().candidates(&context);
        assert_eq!(candidates.values[0].insert, "select");
        assert!(h.pending_request().is_none());
    }
    #[test]
    fn general_context_keeps_keywords_before_a_large_catalog() {
        let (wake, _) = async_channel::bounded(1);
        let h = CompletionHandle::new(Rc::new(Cell::new(0)), wake);
        h.bind(Some("c".into()));
        h.set_connected(true);
        let lease = Lease::admit(h.0.borrow().budget.clone(), CACHE_BYTES).unwrap();
        h.0.borrow_mut().cache = Some(Cache {
            schemas: vec![Schema {
                name: "public".into(),
                relations: (0..200)
                    .map(|i| Relation {
                        name: format!("table{i}"),
                        view: false,
                    })
                    .collect(),
            }],
            columns: None,
            truncated: true,
            _lease: lease,
        });
        let general =
            h.0.borrow_mut()
                .candidates(&context::parse("", "").unwrap());
        assert_eq!(general.values[0].insert, "select");
        assert_eq!(general.values.len(), MENU_ITEMS);
        assert!(h.status().contains("catalog is partial"));
        let relation =
            h.0.borrow_mut()
                .candidates(&context::parse("select * from ", "").unwrap());
        assert_eq!(relation.values[0].insert, "\"table0\"");
    }
}

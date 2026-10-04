//! Plan 031 step 4: the Redis workspace for one saved connection. It owns
//! one backend `RedisSession` and every Tokio job that uses it, renders the
//! sidebar keyspace tree and tab strip through [`RedisPart`] views, and
//! renders the console (always the first tab) and key inspector tabs itself.
//!
//! Contracts: one tree page per database and one console command at a time;
//! jobs are aborted and their late results dropped (by generation) on
//! disconnect, loss or close; a lost session is reported, never reopened.
use crate::{
    accessible_editor::AccessibleEditor,
    controller::Host,
    document_view::ConnectionPhase,
    redis_model::{self, Console, Entry, Keyspace, Pending, Tone, TreeRow},
    style, ui,
};
use dbunk_lib::backend::{
    RedisConsoleOutcome, RedisKeyInspection, RedisKeyValue, RedisOverview, RedisPolicy,
    RedisSession, RedisSessionError,
};
use editor::Editor;
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role,
    SharedString, Subscription, UniformListScrollHandle, WeakEntity, Window, div, prelude::*, px,
    svg, uniform_list,
};
use std::{future::Future, rc::Rc, sync::Arc};

pub enum RedisEvent {
    /// A console command finished; feeds the status bar's last latency.
    Latency(u64),
}

struct KeyTab {
    id: u64,
    db: u8,
    key: String,
    state: KeyState,
}

enum KeyState {
    Loading,
    Loaded(RedisKeyInspection),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Tree,
    Tabs,
}

/// A view onto one part of a [`RedisWorkspace`], so the shell can place the
/// tree in the sidebar and the tabs in its tab bar.
pub struct RedisPart {
    owner: WeakEntity<RedisWorkspace>,
    part: Part,
    _observe: Subscription,
}

impl Render for RedisPart {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.owner
            .update(cx, |owner, cx| match part {
                Part::Tree => owner.tree(window, cx),
                Part::Tabs => owner.tabs(cx),
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

pub struct RedisWorkspace {
    connection_id: String,
    host: Arc<Host>,
    phase: ConnectionPhase,
    session: Option<Arc<RedisSession>>,
    /// Bumped by connect, disconnect and loss; older job results are dropped.
    generation: u64,
    jobs: Vec<tokio::task::AbortHandle>,
    policy: Option<RedisPolicy>,
    version: Option<String>,
    keyspace: Keyspace,
    console: Console,
    rows: Rc<[(SharedString, Tone)]>,
    input: Entity<Editor>,
    input_field: Entity<AccessibleEditor>,
    keys: Vec<KeyTab>,
    /// `None` = the console tab.
    active: Option<u64>,
    next_tab: u64,
    focus: FocusHandle,
    console_scroll: UniformListScrollHandle,
    tree_scroll: UniformListScrollHandle,
    tree_view: Option<Entity<RedisPart>>,
    tabs_view: Option<Entity<RedisPart>>,
}

impl EventEmitter<RedisEvent> for RedisWorkspace {}

impl RedisWorkspace {
    pub fn new(
        connection_id: String,
        host: Arc<Host>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Redis command, e.g. GET key", window, cx);
            editor
        });
        let input_field =
            cx.new(|cx| AccessibleEditor::field(input.clone(), "Redis console command", false, cx));
        Self {
            connection_id,
            host,
            phase: ConnectionPhase::Idle,
            session: None,
            generation: 0,
            jobs: Vec::new(),
            policy: None,
            version: None,
            keyspace: Keyspace::default(),
            console: Console::default(),
            rows: Rc::from(Vec::new()),
            input,
            input_field,
            keys: Vec::new(),
            active: None,
            next_tab: 0,
            focus: cx.focus_handle(),
            console_scroll: UniformListScrollHandle::new(),
            tree_scroll: UniformListScrollHandle::new(),
            tree_view: None,
            tabs_view: None,
        }
    }

    fn part(&mut self, part: Part, cx: &mut Context<Self>) -> Entity<RedisPart> {
        let slot = match part {
            Part::Tree => &self.tree_view,
            Part::Tabs => &self.tabs_view,
        };
        if let Some(view) = slot {
            return view.clone();
        }
        let owner = cx.entity();
        let view = cx.new(|cx| RedisPart {
            owner: owner.downgrade(),
            part,
            _observe: cx.observe(&owner, |_, _, cx| cx.notify()),
        });
        match part {
            Part::Tree => self.tree_view = Some(view.clone()),
            Part::Tabs => self.tabs_view = Some(view.clone()),
        }
        view
    }

    pub fn tree_view(&mut self, cx: &mut Context<Self>) -> Entity<RedisPart> {
        self.part(Part::Tree, cx)
    }

    pub fn tabs_view(&mut self, cx: &mut Context<Self>) -> Entity<RedisPart> {
        self.part(Part::Tabs, cx)
    }

    pub fn phase(&self) -> ConnectionPhase {
        self.phase.clone()
    }

    /// Opens a session unless one is open or opening. Explicit only: the
    /// caller is a user selection, connect or retry.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.phase,
            ConnectionPhase::Connecting | ConnectionPhase::Connected
        ) {
            return;
        }
        self.retire();
        self.phase = ConnectionPhase::Connecting;
        let backend = self.host.backend.clone();
        let id = self.connection_id.clone();
        let generation = self.generation;
        let job = self.host.runtime.spawn(async move {
            let session = Arc::new(backend.open_redis_session(id).await?);
            let overview = session
                .overview()
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((session, overview))
        });
        self.jobs.push(job.abort_handle());
        cx.spawn(async move |this, cx| {
            let Ok(result) = job.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                match result {
                    Ok((session, overview)) => this.opened(session, overview, cx),
                    Err(error) => {
                        this.note(Tone::Error, format!("Connection failed: {error}"));
                        this.phase = ConnectionPhase::Failed(error);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn opened(
        &mut self,
        session: Arc<RedisSession>,
        overview: RedisOverview,
        cx: &mut Context<Self>,
    ) {
        let policy = session.policy();
        self.policy = Some(policy);
        self.version = overview.version;
        self.console.db = overview.default_db;
        self.keyspace = Keyspace::new(&overview.databases, overview.default_db);
        self.session = Some(session);
        self.phase = ConnectionPhase::Connected;
        self.note(
            Tone::Note,
            format!(
                "Connected{}{}",
                self.version
                    .as_ref()
                    .map(|version| format!(" to Redis {version}"))
                    .unwrap_or_default(),
                if policy.read_only {
                    " · read-only"
                } else if policy.confirm_writes {
                    " · writes need confirmation"
                } else {
                    ""
                }
            ),
        );
        self.load_page(overview.default_db, cx);
    }

    /// Closes the session and aborts its jobs; the tree empties, the
    /// transcript and inspector snapshots stay.
    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        if self.session.is_some() || self.phase == ConnectionPhase::Connecting {
            self.note(Tone::Note, "Disconnected".into());
        }
        self.retire();
        self.phase = ConnectionPhase::Idle;
        cx.notify();
    }

    fn retire(&mut self) {
        self.generation += 1;
        for job in self.jobs.drain(..) {
            job.abort();
        }
        self.session = None;
        self.policy = None;
        self.keyspace = Keyspace::default();
        self.console.running = false;
        self.console.pending = None;
    }

    fn lost(&mut self, reason: String, cx: &mut Context<Self>) {
        self.note(Tone::Error, format!("Session lost: {reason}"));
        self.retire();
        self.phase = ConnectionPhase::Failed(reason);
        cx.notify();
    }

    /// Runs `work` on the session in an owned Tokio job; `done` sees the
    /// result only if the session is still the one that started it.
    fn spawn<T, F>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(Arc<RedisSession>) -> F,
        done: impl FnOnce(&mut Self, Result<T, RedisSessionError>, &mut Context<Self>) + 'static,
    ) -> bool
    where
        T: Send + 'static,
        F: Future<Output = Result<T, RedisSessionError>> + Send + 'static,
    {
        let Some(session) = self.session.clone() else {
            return false;
        };
        let job = self.host.runtime.spawn(work(session));
        self.jobs.retain(|job| !job.is_finished());
        self.jobs.push(job.abort_handle());
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let Ok(result) = job.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                let lost = match &result {
                    Err(RedisSessionError::Lost(reason)) => Some(reason.clone()),
                    _ => None,
                };
                done(this, result, cx);
                if let Some(reason) = lost {
                    this.lost(reason, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        true
    }

    fn load_page(&mut self, db: u8, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        let Some(cursor) = self.keyspace.get_mut(db).and_then(|node| node.begin_page()) else {
            return;
        };
        let retry = cursor.clone();
        let started = self.spawn(
            cx,
            move |session| async move { session.scan(db, cursor, "*").await },
            move |this, result, _| {
                if let Some(node) = this.keyspace.get_mut(db) {
                    match result {
                        Ok(page) => node.apply_page(page),
                        Err(error) => node.fail_page(retry, error.to_string()),
                    }
                }
            },
        );
        if !started && let Some(node) = self.keyspace.get_mut(db) {
            node.fail_page(None, "Not connected".into());
        }
        cx.notify();
    }

    /// Re-reads database totals and restarts sampling from the first page.
    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        self.spawn(
            cx,
            |session| async move { session.overview().await },
            |this, result, cx| {
                if let Ok(overview) = result {
                    let expanded: Vec<u8> = this
                        .keyspace
                        .databases
                        .iter()
                        .filter(|db| db.expanded)
                        .map(|db| db.index)
                        .collect();
                    this.keyspace = Keyspace::new(&overview.databases, overview.default_db);
                    for db in expanded {
                        if let Some(node) = this.keyspace.get_mut(db) {
                            node.expanded = true;
                        }
                        this.load_page(db, cx);
                    }
                }
            },
        );
    }

    fn toggle_db(&mut self, db: u8, cx: &mut Context<Self>) {
        let Some(node) = self.keyspace.get_mut(db) else {
            return;
        };
        node.expanded = !node.expanded;
        let load = node.expanded && node.scan == redis_model::ScanState::NotStarted;
        if load {
            self.load_page(db, cx);
        }
        cx.notify();
    }

    fn toggle_group(&mut self, db: u8, kind: &str, cx: &mut Context<Self>) {
        if let Some(node) = self.keyspace.get_mut(db)
            && !node.open_groups.remove(kind)
        {
            node.open_groups.insert(kind.to_owned());
        }
        cx.notify();
    }

    fn note(&mut self, tone: Tone, text: String) {
        self.console.push(Entry {
            db: self.console.db,
            command: String::new(),
            lines: vec![text],
            tone,
            elapsed_ms: None,
        });
        self.rebuild_rows();
    }

    /// Flattens the newest entries into at most `CONSOLE_ROWS` lines.
    fn rebuild_rows(&mut self) {
        let mut rows = Vec::new();
        for entry in self.console.entries.iter().rev() {
            if rows.len() >= redis_model::CONSOLE_ROWS {
                break;
            }
            if let Some(ms) = entry.elapsed_ms {
                rows.push((SharedString::from(format!("({ms} ms)")), Tone::Note));
            }
            rows.extend(
                entry
                    .lines
                    .iter()
                    .rev()
                    .map(|line| (SharedString::from(line.clone()), entry.tone)),
            );
            if !entry.command.is_empty() {
                rows.push((
                    SharedString::from(format!("db{}> {}", entry.db, entry.command)),
                    Tone::Note,
                ));
            }
        }
        rows.truncate(redis_model::CONSOLE_ROWS);
        rows.reverse();
        let count = rows.len();
        self.rows = Rc::from(rows);
        self.console_scroll
            .scroll_to_item(count.saturating_sub(1), gpui::ScrollStrategy::Bottom);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.console.running {
            return;
        }
        let input = self.input.read(cx).text(cx);
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.eq_ignore_ascii_case("clear") {
            self.clear_console(window, cx);
            return;
        }
        match redis_model::tokenize(trimmed) {
            Err(error) => {
                self.note(Tone::Error, error);
                cx.notify();
            }
            Ok(tokens) => {
                self.console.remember(trimmed);
                self.input
                    .update(cx, |editor, cx| editor.set_text("", window, cx));
                self.run(trimmed.to_owned(), tokens, false, cx);
            }
        }
    }

    fn run(&mut self, input: String, tokens: Vec<String>, confirmed: bool, cx: &mut Context<Self>) {
        self.console.pending = None;
        let command = input.clone();
        let retry = tokens.clone();
        let db = self.console.db;
        let started = self.spawn(
            cx,
            move |session| async move { session.run(tokens, confirmed).await },
            move |this, result, cx| {
                this.console.running = false;
                let entry = |lines, tone, elapsed_ms| Entry {
                    db,
                    command: command.clone(),
                    lines,
                    tone,
                    elapsed_ms,
                };
                match result {
                    Ok(RedisConsoleOutcome::Reply {
                        value,
                        truncated,
                        elapsed_ms,
                        db: now,
                    }) => {
                        let mut lines = redis_model::reply_lines(&value);
                        if truncated {
                            lines.push("(reply cut to the console's size limit)".into());
                        }
                        let tone = if matches!(value, dbunk_lib::backend::RedisValue::Error(_)) {
                            Tone::Error
                        } else {
                            Tone::Reply
                        };
                        this.console.push(entry(lines, tone, Some(elapsed_ms)));
                        this.console.db = now;
                        cx.emit(RedisEvent::Latency(elapsed_ms));
                    }
                    Ok(RedisConsoleOutcome::NeedsConfirmation {
                        command: name,
                        reason,
                    }) => {
                        this.console.push(entry(
                            vec![format!("{reason}. Confirm to run {name}.")],
                            Tone::Note,
                            None,
                        ));
                        this.console.pending = Some(Pending {
                            input: command.clone(),
                            tokens: retry,
                            command: name,
                            reason,
                        });
                    }
                    Ok(RedisConsoleOutcome::Refused { reason }) => {
                        this.console.push(entry(vec![reason], Tone::Error, None));
                    }
                    Err(error) => {
                        this.console
                            .push(entry(vec![error.to_string()], Tone::Error, None));
                    }
                }
                this.rebuild_rows();
            },
        );
        if started {
            self.console.running = true;
        } else {
            self.note(
                Tone::Error,
                "Not connected. Select the connection to connect.".into(),
            );
        }
        cx.notify();
    }

    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(pending) = self.console.pending.take() {
            self.run(pending.input, pending.tokens, true, cx);
        }
    }

    fn cancel_pending(&mut self, cx: &mut Context<Self>) {
        if self.console.pending.take().is_some() {
            self.note(Tone::Note, "Cancelled".into());
        }
        cx.notify();
    }

    pub fn clear_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.console.entries.clear();
        self.input
            .update(cx, |editor, cx| editor.set_text("", window, cx));
        self.rebuild_rows();
        cx.notify();
    }

    fn input_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.focus_handle(cx).contains_focused(window, cx) {
            return;
        }
        let composing = self.input.update(cx, |editor, cx| {
            gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
        });
        let modifiers = &event.keystroke.modifiers;
        if composing || modifiers.control || modifiers.alt || modifiers.platform {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" => self.submit(window, cx),
            "up" | "down" => {
                let Some(text) = self.console.recall(event.keystroke.key == "up") else {
                    return;
                };
                self.input
                    .update(cx, |editor, cx| editor.set_text(text, window, cx));
            }
            "escape" if self.console.pending.is_some() => self.cancel_pending(cx),
            _ => return,
        }
        cx.stop_propagation();
    }

    /// Opens (or selects) the inspector tab for a UTF-8 key.
    fn inspect(&mut self, db: u8, name: &[u8], window: &mut Window, cx: &mut Context<Self>) {
        let Ok(key) = String::from_utf8(name.to_vec()) else {
            self.note(
                Tone::Error,
                format!(
                    "{} is not UTF-8; inspect it from the console",
                    redis_model::display_key(name)
                ),
            );
            self.select_tab(None, window, cx);
            return;
        };
        if let Some(tab) = self.keys.iter().find(|tab| tab.db == db && tab.key == key) {
            let id = tab.id;
            self.select_tab(Some(id), window, cx);
            return;
        }
        if self.keys.len() == redis_model::INSPECTORS {
            // Replace the oldest inspector rather than grow without bound.
            let oldest = self.keys.remove(0);
            if self.active == Some(oldest.id) {
                self.active = None;
            }
        }
        let id = self.next_tab;
        self.next_tab += 1;
        self.keys.push(KeyTab {
            id,
            db,
            key,
            state: KeyState::Loading,
        });
        self.select_tab(Some(id), window, cx);
        self.load_key(id, cx);
    }

    fn load_key(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(tab) = self.keys.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        tab.state = KeyState::Loading;
        let (db, key) = (tab.db, tab.key.clone());
        let started = self.spawn(
            cx,
            move |session| async move { session.inspect(db, key).await },
            move |this, result, _| {
                if let Some(tab) = this.keys.iter_mut().find(|tab| tab.id == id) {
                    tab.state = match result {
                        Ok(inspection) => KeyState::Loaded(inspection),
                        Err(error) => KeyState::Failed(error.to_string()),
                    };
                }
            },
        );
        if !started && let Some(tab) = self.keys.iter_mut().find(|tab| tab.id == id) {
            tab.state = KeyState::Failed("Not connected".into());
        }
        cx.notify();
    }

    fn select_tab(&mut self, id: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        self.active = id;
        self.focus_active(window, cx);
        cx.notify();
    }

    pub fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.active {
            self.close_key(id, window, cx);
        }
    }

    fn close_key(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.keys.iter().position(|tab| tab.id == id) else {
            return;
        };
        self.keys.remove(index);
        if self.active == Some(id) {
            let next = self
                .keys
                .get(index.min(self.keys.len().saturating_sub(1)))
                .map(|tab| tab.id);
            self.select_tab(next, window, cx);
        }
        cx.notify();
    }

    /// Next/previous tab, console included.
    pub fn cycle_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order: Vec<Option<u64>> = std::iter::once(None)
            .chain(self.keys.iter().map(|tab| Some(tab.id)))
            .collect();
        let at = order.iter().position(|id| *id == self.active).unwrap_or(0);
        let next = if forward {
            (at + 1) % order.len()
        } else {
            (at + order.len() - 1) % order.len()
        };
        self.select_tab(order[next], window, cx);
    }

    pub fn show_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.select_tab(None, window, cx);
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if self.active.is_none() {
            window.focus(&self.input.focus_handle(cx), cx);
        } else {
            window.focus(&self.focus, cx);
        }
    }

    fn tree(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let header = div()
            .flex_none()
            .h(px(style::ROW + 4.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(style::FONT_SMALL))
            .text_color(style::faint())
            .child(div().flex_1().child("KEYSPACE"))
            .when(self.session.is_some(), |header| {
                header.child(
                    ui::tool_button(
                        "redis-refresh-tree",
                        "Refresh",
                        Some("icons/rotate_cw.svg"),
                        true,
                        false,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_tree(cx))),
                )
            });
        let body = match &self.phase {
            ConnectionPhase::Connected => {
                let rows = Rc::new(self.keyspace.rows());
                let count = rows.len();
                uniform_list(
                    "redis-tree-rows",
                    count,
                    cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| this.tree_row(index, &rows[index], cx))
                            .collect()
                    }),
                )
                .track_scroll(&self.tree_scroll)
                .flex_1()
                .into_any_element()
            }
            phase => div()
                .px(px(10.))
                .py(px(6.))
                .text_size(px(style::FONT_SMALL))
                .text_color(match phase {
                    ConnectionPhase::Failed(_) => style::bad(),
                    _ => style::faint(),
                })
                .child(match phase {
                    ConnectionPhase::Connecting => "Connecting…".to_owned(),
                    ConnectionPhase::Failed(error) => format!("Not connected: {error}"),
                    _ => "Not connected".to_owned(),
                })
                .into_any_element(),
        };
        div()
            .id("redis-keyspace")
            .role(Role::Tree)
            .aria_label("Redis keyspace")
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }

    fn tree_row(&self, index: usize, row: &TreeRow, cx: &mut Context<Self>) -> AnyElement {
        let base = |depth: f32| {
            div()
                .id(("redis-tree-row", index))
                .role(Role::TreeItem)
                .h(px(style::ROW))
                .pl(px(10. + depth * 12.))
                .pr(px(8.))
                .flex()
                .items_center()
                .gap(px(5.))
                .text_size(px(style::FONT))
                .whitespace_nowrap()
                .overflow_hidden()
        };
        let chevron = |open: bool| {
            svg()
                .path(if open {
                    "icons/chevron_down.svg"
                } else {
                    "icons/chevron_right.svg"
                })
                .size(px(style::ICON))
                .flex_none()
                .text_color(style::faint())
        };
        let count = |text: String| {
            div()
                .flex_none()
                .text_size(px(style::FONT_SMALL))
                .text_color(style::faint())
                .child(text)
        };
        match row.clone() {
            TreeRow::Db {
                index: db,
                total,
                expanded,
                loading,
            } => base(0.)
                .aria_label(format!("db{db}, {total} keys"))
                .aria_expanded(expanded)
                .cursor_pointer()
                .hover(|s| s.bg(style::row_hover()))
                .text_color(if total == 0 {
                    style::dim()
                } else {
                    style::text()
                })
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_db(db, cx)))
                .child(chevron(expanded))
                .child(div().flex_1().child(format!("db{db}")))
                .child(count(if loading {
                    "scanning…".into()
                } else {
                    format!("{total} keys")
                }))
                .into_any_element(),
            TreeRow::Group {
                db,
                kind,
                count: estimate,
                open,
            } => {
                let label = estimate.label();
                let words = match estimate {
                    redis_model::Count::Exact(_) => format!("{kind}, {label} keys"),
                    redis_model::Count::Estimate(_) => {
                        format!(
                            "{kind}, about {} keys, estimated from a sample",
                            &label[1..]
                        )
                    }
                };
                let toggled = kind.clone();
                base(1.)
                    .aria_label(words)
                    .aria_expanded(open)
                    .cursor_pointer()
                    .hover(|s| s.bg(style::row_hover()))
                    .text_color(style::dim())
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_group(db, &toggled, cx)),
                    )
                    .child(chevron(open))
                    .child(div().flex_1().child(kind))
                    .child(count(label))
                    .into_any_element()
            }
            TreeRow::Key { db, name } => {
                let label = redis_model::display_key(&name);
                base(2.)
                    .aria_label(label.clone())
                    .cursor_pointer()
                    .hover(|s| s.bg(style::row_hover()))
                    .font_family(style::MONO)
                    .text_size(px(style::FONT_SMALL))
                    .text_color(style::text())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.inspect(db, &name, window, cx)),
                    )
                    .child(div().pl(px(style::ICON + 5.)).child(label))
                    .into_any_element()
            }
            TreeRow::Hidden { count: hidden, .. } => base(2.)
                .text_size(px(style::FONT_SMALL))
                .text_color(style::faint())
                .child(div().pl(px(style::ICON + 5.)).child(format!(
                    "{hidden} more loaded; use SCAN in the console to list them"
                )))
                .into_any_element(),
            TreeRow::More { db, sampled, total } => base(1.)
                .role(Role::Button)
                .aria_label(format!("Load more keys from db{db}"))
                .cursor_pointer()
                .hover(|s| s.bg(style::row_hover()))
                .text_size(px(style::FONT_SMALL))
                .text_color(style::accent())
                .on_click(cx.listener(move |this, _, _, cx| this.load_page(db, cx)))
                .child(format!("Load more · {sampled} of {total} sampled"))
                .into_any_element(),
            TreeRow::Error { message, .. } => base(1.)
                .role(Role::Alert)
                .aria_label(message.clone())
                .text_size(px(style::FONT_SMALL))
                .text_color(style::bad())
                .child(message)
                .into_any_element(),
        }
    }

    fn tabs(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let tab = |id: SharedString, label: String, icon: &'static str, active: bool| {
            div()
                .id(id)
                .role(Role::Tab)
                .aria_label(label.clone())
                .aria_selected(active)
                .h_full()
                .min_w(px(110.))
                .max_w(px(220.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .border_r_1()
                .border_color(style::line_soft())
                .cursor_pointer()
                .text_color(if active { style::text() } else { style::dim() })
                .when(active, |tab| tab.bg(style::bg()))
                .hover(|s| s.text_color(style::text()))
                .child(
                    svg()
                        .path(icon)
                        .size(px(style::ICON))
                        .flex_none()
                        .text_color(if active { style::text() } else { style::dim() }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(label),
                )
        };
        let mut tabs = vec![
            tab(
                "redis-tab-console".into(),
                "Console".into(),
                "icons/terminal.svg",
                self.active.is_none(),
            )
            .on_click(cx.listener(|this, _, window, cx| this.select_tab(None, window, cx)))
            .into_any_element(),
        ];
        for key in &self.keys {
            let id = key.id;
            let label = format!("{} · db{}", key.key, key.db);
            tabs.push(
                tab(
                    SharedString::from(format!("redis-tab-{id}")),
                    label.clone(),
                    "icons/hash.svg",
                    self.active == Some(id),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.select_tab(Some(id), window, cx)),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("redis-close-tab-{id}")))
                        .role(Role::Button)
                        .aria_label(format!("Close {label}"))
                        .size(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(4.))
                        .hover(|s| s.bg(style::hover()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_key(id, window, cx)
                        }))
                        .child(
                            svg()
                                .path("icons/close.svg")
                                .size(px(style::ICON))
                                .text_color(style::dim()),
                        ),
                )
                .into_any_element(),
            );
        }
        div()
            .id("redis-tabs")
            .role(Role::TabList)
            .aria_label("Redis tabs")
            .h_full()
            .flex()
            .min_w_0()
            .overflow_x_scroll()
            .children(tabs)
            .into_any_element()
    }

    fn console_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let rows = self.rows.clone();
        let count = rows.len();
        let connected = self.session.is_some();
        let policy = self.policy;
        let toolbar = ui::toolbar()
            .child(
                div()
                    .text_color(style::text())
                    .child(format!("Console · db{}", self.console.db)),
            )
            .when_some(policy, |bar, policy| {
                bar.when(policy.read_only, |bar| bar.child(ui::badge("read-only")))
                    .when(!policy.read_only && policy.confirm_writes, |bar| {
                        bar.child(ui::badge("writes confirmed"))
                    })
            })
            .child(ui::grow())
            .when(self.console.running, |bar| bar.child("Running…"))
            .child(
                ui::tool_button("redis-console-clear", "Clear", None, count > 0, false)
                    .on_click(cx.listener(|this, _, window, cx| this.clear_console(window, cx))),
            );
        let transcript = div()
            .id("redis-console-transcript")
            .role(Role::Log)
            .aria_label(format!("{count} console lines"))
            .flex_1()
            .min_h_0()
            .child(
                uniform_list(
                    "redis-console-rows",
                    count,
                    cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                        range
                            .map(|index| {
                                let (text, tone) = rows[index].clone();
                                div()
                                    .id(("redis-console-row", index))
                                    .h(px(style::ROW - 4.))
                                    .px(px(10.))
                                    .font_family(style::MONO)
                                    .text_size(px(style::FONT_SMALL))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_color(match tone {
                                        Tone::Reply => style::text(),
                                        Tone::Error => style::bad(),
                                        Tone::Note => style::dim(),
                                    })
                                    .child(text)
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.console_scroll)
                .h_full(),
            );
        let pending = self.console.pending.clone().map(|pending| {
            div()
                .id("redis-confirm")
                .role(Role::Alert)
                .aria_label(format!("{}. Run {}?", pending.reason, pending.command))
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .bg(style::bad_fill())
                .border_t_1()
                .border_color(style::bad_line())
                .text_color(style::bad_text())
                .child(div().flex_1().child(pending.reason.clone()))
                .child(
                    ui::button("redis-confirm-cancel", "Cancel", ui::Variant::Ghost, true)
                        .on_click(cx.listener(|this, _, _, cx| this.cancel_pending(cx))),
                )
                .child(
                    ui::button(
                        "redis-confirm-run",
                        format!("Run {}", pending.command),
                        ui::Variant::Danger,
                        connected,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.confirm(cx))),
                )
        });
        let prompt = div()
            .flex_none()
            .h(px(style::TOOLBAR))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(6.))
            .border_t_1()
            .border_color(style::line_soft())
            .font_family(style::MONO)
            .text_size(px(style::FONT))
            .capture_key_down(cx.listener(Self::input_key))
            .child(
                div()
                    .flex_none()
                    .text_color(if connected {
                        style::accent()
                    } else {
                        style::faint()
                    })
                    .child(format!("db{}>", self.console.db)),
            )
            .child(div().flex_1().min_w_0().child(self.input_field.clone()));
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(transcript)
            .children(pending)
            .child(prompt)
            .into_any_element()
    }

    fn key_view(&mut self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let Some(tab) = self.keys.iter().find(|tab| tab.id == id) else {
            return div().into_any_element();
        };
        let connected = self.session.is_some();
        let title = format!("{} · db{}", tab.key, tab.db);
        let mut toolbar = ui::toolbar()
            .child(
                div()
                    .font_family(style::MONO)
                    .text_color(style::text())
                    .child(title),
            )
            .child(ui::grow());
        let body: AnyElement = match &tab.state {
            KeyState::Loading => div()
                .p(px(10.))
                .text_color(style::faint())
                .child("Loading…")
                .into_any_element(),
            KeyState::Failed(error) => div()
                .p(px(10.))
                .child(ui::error_banner(("redis-key-error", id), error.clone()))
                .into_any_element(),
            KeyState::Loaded(inspection) => {
                toolbar = toolbar
                    .child(ui::badge(inspection.kind.clone()))
                    .child(format!(
                        "TTL {}",
                        redis_model::ttl_label(inspection.ttl_seconds)
                    ))
                    .when_some(inspection.encoding.clone(), |bar, encoding| {
                        bar.child(encoding)
                    })
                    .when_some(inspection.length, |bar, length| {
                        bar.child(if inspection.kind == "string" {
                            format!("{length} bytes")
                        } else {
                            format!("{length} items")
                        })
                    });
                let rows: Rc<Vec<(String, String)>> =
                    Rc::new(redis_model::inspector_rows(inspection));
                let count = rows.len();
                let note = match &inspection.value {
                    RedisKeyValue::Missing => Some("The key no longer exists".to_owned()),
                    _ if inspection.truncated => Some(format!(
                        "Showing the first {} {}; use the console for the rest",
                        count,
                        if inspection.kind == "string" {
                            "bytes"
                        } else {
                            "items"
                        }
                    )),
                    _ => None,
                };
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when_some(note, |body, note| {
                        body.child(
                            div()
                                .flex_none()
                                .px(px(10.))
                                .py(px(4.))
                                .text_size(px(style::FONT_SMALL))
                                .text_color(style::faint())
                                .child(note),
                        )
                    })
                    .child(
                        uniform_list(
                            ("redis-key-rows", id),
                            count,
                            cx.processor(move |_, range: std::ops::Range<usize>, _, _| {
                                range
                                    .map(|index| {
                                        let (label, value) = rows[index].clone();
                                        div()
                                            .id(("redis-key-row", index))
                                            .role(Role::Row)
                                            .aria_label(format!("{label} {value}"))
                                            .h(px(style::ROW))
                                            .px(px(10.))
                                            .flex()
                                            .items_center()
                                            .gap(px(12.))
                                            .border_b_1()
                                            .border_color(style::line_soft())
                                            .font_family(style::MONO)
                                            .text_size(px(style::FONT_SMALL))
                                            .whitespace_nowrap()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .w(px(160.))
                                                    .flex_none()
                                                    .overflow_hidden()
                                                    .text_color(style::dim())
                                                    .child(label),
                                            )
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_color(style::text())
                                                    .child(value),
                                            )
                                    })
                                    .collect()
                            }),
                        )
                        .flex_1(),
                    )
                    .into_any_element()
            }
        };
        let toolbar = toolbar.child(
            ui::tool_button(
                ("redis-key-refresh", id),
                "Refresh",
                Some("icons/rotate_cw.svg"),
                connected && !matches!(tab.state, KeyState::Loading),
                false,
            )
            .when(connected, |button| {
                button.on_click(cx.listener(move |this, _, _, cx| this.load_key(id, cx)))
            }),
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}

impl Focusable for RedisWorkspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RedisWorkspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.active {
            None => self.console_view(cx),
            Some(id) => self.key_view(id, cx),
        };
        div()
            .id("redis-workspace")
            .role(Role::Group)
            .aria_label("Redis workspace")
            .track_focus(&self.focus)
            .size_full()
            .bg(style::bg())
            .child(body)
    }
}

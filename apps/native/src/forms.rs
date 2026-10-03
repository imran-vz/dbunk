//! Native forms share one editing model, secure AX fields and deterministic
//! keyboard return. Stored passwords are never loaded into an editor.
use crate::{accessible_editor::AccessibleEditor, controller::Host};
use dbunk_lib::backend::*;
use editor::Editor;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, PathPromptOptions,
    Role, SharedString, Task, Window,
    accesskit::{Action, Live, Toggled},
    div,
    prelude::*,
    px, rgb,
};
use std::{collections::HashMap, sync::Arc};

mod diagnosis;
mod uri;

type FormJob =
    futures_util::future::BoxFuture<'static, (Option<String>, Result<Option<String>, String>)>;

pub enum FormEvent {
    Saved,
    Cancelled,
    Renamed(String),
    OpenTable { schema: String, table: String },
    ConfirmedDiscard,
}
#[derive(Clone, Copy)]
enum FormAction {
    Submit,
    ImportUri,
    Test,
    Cancel,
    Reset,
    ConfirmReset,
    Recover,
    Discard,
    Delete,
    Mode(DevelopmentStorageMode),
    Environment(DevelopmentEnvironment),
    Safe(DevelopmentSafeMode),
    Tls(DevelopmentTlsMode),
    ReadOnly,
    Favorite,
    Pick(&'static str),
}
enum Kind {
    Credentials(DevelopmentSettings),
    Connection { id: Option<String> },
    Rename,
    OpenTable,
    Delete(String),
    Discard,
    ResetWorkspace,
}
struct Field {
    key: &'static str,
    label: &'static str,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}
pub struct Form {
    host: Arc<Host>,
    kind: Kind,
    fields: Vec<Field>,
    controls: HashMap<String, FocusHandle>,
    visible_controls: Vec<FocusHandle>,
    mode: DevelopmentStorageMode,
    environment: DevelopmentEnvironment,
    safe: DevelopmentSafeMode,
    tls: DevelopmentTlsMode,
    read_only: bool,
    favorite: bool,
    busy: bool,
    confirm_reset: bool,
    message: Option<String>,
    task: Option<Task<()>>,
    diagnosis: Option<diagnosis::State>,
}
impl EventEmitter<FormEvent> for Form {}
impl Form {
    fn base(host: Arc<Host>, kind: Kind, cx: &Context<Self>) -> Self {
        Self {
            host,
            kind,
            fields: Vec::new(),
            controls: HashMap::from([("Cancel".into(), cx.focus_handle())]),
            visible_controls: Vec::new(),
            mode: DevelopmentStorageMode::PlainSqlite,
            environment: DevelopmentEnvironment::Development,
            safe: DevelopmentSafeMode::Protected,
            tls: DevelopmentTlsMode::Disable,
            read_only: false,
            favorite: false,
            busy: false,
            confirm_reset: false,
            message: None,
            task: None,
            diagnosis: None,
        }
    }
    fn field(
        &mut self,
        key: &'static str,
        label: &'static str,
        value: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(value, window, cx);
            editor.set_masked(secret, cx);
            editor
        });
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, secret, cx));
        cx.subscribe(&editor, |this, _, event, cx| {
            if matches!(event, editor::EditorEvent::BufferEdited)
                && let Some(state) = &mut this.diagnosis
            {
                state.invalidate();
                this.message = None;
                cx.notify();
            }
        })
        .detach();
        self.fields.push(Field {
            key,
            label,
            editor,
            accessible,
        });
    }
    pub fn credentials(
        host: Arc<Host>,
        settings: DevelopmentSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mode = settings.mode.unwrap_or(DevelopmentStorageMode::PlainSqlite);
        let recovery = settings.state == DevelopmentCredentialState::NeedsRecovery;
        let mut form = Self::base(host, Kind::Credentials(settings), cx);
        form.mode = mode;
        if !recovery {
            form.field(
                "password",
                "Credential password",
                String::new(),
                true,
                window,
                cx,
            );
        }
        form.focus(window, cx);
        form
    }
    pub fn connection(
        host: Arc<Host>,
        connection: Option<DevelopmentConnection>,
        retained: std::rc::Rc<std::cell::Cell<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let data = connection
            .as_ref()
            .and_then(|connection| connection.postgres.clone())
            .unwrap_or_else(|| connection_defaults(host.backend.native_profile_kind()));
        let mut form = Self::base(
            host,
            Kind::Connection {
                id: connection.as_ref().map(|connection| connection.id.clone()),
            },
            cx,
        );
        form.diagnosis = Some(diagnosis::State::new(retained));
        form.environment = data.environment;
        form.safe = data.safe_mode;
        form.tls = data.tls.mode;
        form.read_only = data.read_only;
        form.favorite = connection
            .as_ref()
            .is_some_and(|connection| connection.organization.is_favorite);
        for (key, label, value) in [
            ("name", "Name", data.name),
            ("host", "Host", data.host),
            ("port", "Port", data.port.to_string()),
            ("database", "Database", data.database),
            ("user", "User", data.user),
            ("password", "Database password", String::new()),
            (
                "folder",
                "Folder",
                connection
                    .as_ref()
                    .map(|c| c.organization.folder.clone())
                    .unwrap_or_default(),
            ),
            (
                "color",
                "Color",
                connection
                    .as_ref()
                    .map(|c| c.organization.color.clone())
                    .unwrap_or_default(),
            ),
            (
                "root-cert",
                "Root certificate",
                data.tls.root_cert_path.unwrap_or_default(),
            ),
            (
                "client-cert",
                "Client certificate",
                data.tls.client_cert_path.unwrap_or_default(),
            ),
            (
                "client-key",
                "Client key",
                data.tls.client_key_path.unwrap_or_default(),
            ),
            (
                "server-name",
                "TLS server name",
                data.tls.server_name.unwrap_or_default(),
            ),
            (
                "statement-timeout",
                "Statement timeout (ms)",
                number(data.driver_options.statement_timeout_ms),
            ),
            (
                "idle-timeout",
                "Idle transaction timeout (ms)",
                number(data.driver_options.idle_in_transaction_timeout_ms),
            ),
            (
                "connect-timeout",
                "Connect timeout (ms)",
                number(data.driver_options.connect_timeout_ms),
            ),
            (
                "keepalive",
                "Keepalive (seconds)",
                number(data.driver_options.keepalive_seconds),
            ),
            (
                "search-path",
                "Search path (comma separated)",
                data.driver_options
                    .default_search_path
                    .unwrap_or_default()
                    .join(", "),
            ),
            (
                "role",
                "Default role",
                data.driver_options.default_role.unwrap_or_default(),
            ),
        ] {
            form.field(key, label, value, key == "password", window, cx);
        }
        form.focus(window, cx);
        form
    }
    pub fn rename(
        host: Arc<Host>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self::base(host, Kind::Rename, cx);
        form.field("name", "Query name", name, false, window, cx);
        form.focus(window, cx);
        form
    }
    pub fn open_table(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::OpenTable, cx);
        form.field("schema", "Schema", "public".into(), false, window, cx);
        form.field("table", "Table", String::new(), false, window, cx);
        form.focus(window, cx);
        form
    }
    pub fn delete_connection(
        host: Arc<Host>,
        connection: DevelopmentConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut form = Self::base(host, Kind::Delete(connection.id), cx);
        form.message = Some(format!(
            "Delete {}? Saved query drafts will be kept, disconnected.",
            connection.name
        ));
        form.focus(window, cx);
        form
    }
    pub fn discard_drafts(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::Discard, cx);
        form.message =
            Some("Discard unsaved drafts and close? The last saved workspace will be kept.".into());
        form.focus(window, cx);
        form
    }
    pub fn reset_workspace(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::ResetWorkspace, cx);
        form.message = Some("Reset saved query drafts? Connections are kept and the workspace stays open. Export the saved JSON first if you need to preserve it.".into());
        form.focus(window, cx);
        form
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(field) = self.fields.first() {
            window.focus(&field.editor.focus_handle(cx), cx);
        } else if let Some(cancel) = self.controls.get("Cancel") {
            window.focus(cancel, cx);
        }
    }
    fn value(&self, key: &str, cx: &App) -> String {
        self.fields
            .iter()
            .find(|field| field.key == key)
            .map(|field| field.editor.read(cx).text(cx))
            .unwrap_or_default()
    }
    fn connection_input(&self, cx: &App) -> Result<DevelopmentPostgresConnection, String> {
        let optional = |key| {
            let value = self.value(key, cx);
            if value.trim().is_empty() {
                None
            } else {
                Some(value.trim().to_owned())
            }
        };
        let numeric = |key| -> Result<Option<u32>, String> {
            optional(key)
                .map(|value| {
                    value
                        .parse()
                        .map_err(|_| format!("Invalid numeric value for {key}"))
                })
                .transpose()
        };
        let port = self
            .value("port", cx)
            .parse::<u16>()
            .map_err(|_| "Port must be between 1 and 65535")?;
        if port == 0 {
            return Err("Port must be between 1 and 65535".into());
        }
        Ok(DevelopmentPostgresConnection {
            name: self.value("name", cx),
            host: self.value("host", cx),
            port,
            database: self.value("database", cx),
            user: self.value("user", cx),
            environment: self.environment,
            safe_mode: self.safe,
            read_only: self.read_only,
            tls: DevelopmentTlsOptions {
                mode: self.tls,
                root_cert_path: optional("root-cert"),
                client_cert_path: optional("client-cert"),
                client_key_path: optional("client-key"),
                server_name: optional("server-name"),
            },
            driver_options: DevelopmentDriverOptions {
                statement_timeout_ms: numeric("statement-timeout")?,
                idle_in_transaction_timeout_ms: numeric("idle-timeout")?,
                connect_timeout_ms: numeric("connect-timeout")?,
                keepalive_seconds: numeric("keepalive")?,
                default_search_path: optional("search-path").map(|value| {
                    value
                        .split(',')
                        .map(|part| part.trim().to_owned())
                        .collect()
                }),
                default_role: optional("role"),
            },
        })
    }
    fn activate(&mut self, action: FormAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            if matches!(action, FormAction::Cancel)
                && let Some(state) = &self.diagnosis
                && state.running()
            {
                state.cancel();
                self.message = Some("Cancelling connection test…".into());
                cx.notify();
            }
            return;
        }
        if matches!(action, FormAction::Test) {
            self.test_connection(cx);
            return;
        }
        if let Some(state) = &mut self.diagnosis {
            state.invalidate();
            self.message = None;
        }
        match action {
            FormAction::ImportUri => {
                self.import_uri(window, cx);
                return;
            }
            FormAction::Cancel => {
                cx.emit(FormEvent::Cancelled);
                return;
            }
            FormAction::Discard => {
                cx.emit(FormEvent::ConfirmedDiscard);
                return;
            }
            FormAction::Mode(mode) => {
                self.mode = mode;
                cx.notify();
                return;
            }
            FormAction::Environment(value) => {
                self.environment = value;
                cx.notify();
                return;
            }
            FormAction::Safe(value) => {
                self.safe = value;
                cx.notify();
                return;
            }
            FormAction::Tls(value) => {
                self.tls = value;
                cx.notify();
                return;
            }
            FormAction::ReadOnly => {
                self.read_only = !self.read_only;
                cx.notify();
                return;
            }
            FormAction::Favorite => {
                self.favorite = !self.favorite;
                cx.notify();
                return;
            }
            FormAction::Reset => {
                self.confirm_reset = true;
                cx.notify();
                return;
            }
            FormAction::Pick(key) => {
                let picker = cx.prompt_for_paths(PathPromptOptions {
                    files: true,
                    directories: false,
                    multiple: false,
                    prompt: None,
                });
                self.task = Some(cx.spawn_in(window, async move |this, cx| {
                    if let Ok(Ok(Some(paths))) = picker.await
                        && let Some(path) = paths.first()
                    {
                        let text = path.to_string_lossy().into_owned();
                        let _ = this.update_in(cx, |this, window, cx| {
                            if let Some(field) = this.fields.iter().find(|field| field.key == key) {
                                field
                                    .editor
                                    .update(cx, |editor, cx| editor.set_text(text, window, cx));
                            }
                            cx.notify();
                        });
                    }
                }));
                return;
            }
            _ => {}
        }
        if matches!(self.kind, Kind::Rename) {
            let name = self.value("name", cx);
            if name.trim().is_empty() {
                self.message = Some("Enter a query name".into());
                cx.notify();
                return;
            }
            cx.emit(FormEvent::Renamed(name));
            return;
        }
        if matches!(self.kind, Kind::OpenTable) {
            let schema = self.value("schema", cx);
            let table = self.value("table", cx);
            if [&schema, &table]
                .iter()
                .any(|value| value.is_empty() || value.len() > 256 || value.contains('\0'))
            {
                self.message =
                    Some("Enter a schema and table name (at most 256 bytes each)".into());
                cx.notify();
            } else {
                cx.emit(FormEvent::OpenTable { schema, table });
            }
            return;
        }
        let backend = self.host.backend.clone();
        let password = self.value("password", cx);
        let mode = self.mode;
        let job: FormJob = match &self.kind {
            Kind::Connection { id } => {
                let form = match self.connection_input(cx) {
                    Ok(form) => form,
                    Err(error) => {
                        self.message = Some(error);
                        cx.notify();
                        return;
                    }
                };
                let id = id.clone();
                let organization = DevelopmentConnectionOrganization {
                    folder: self.value("folder", cx),
                    is_favorite: self.favorite,
                    color: self.value("color", cx),
                };
                Box::pin(async move {
                    match backend
                        .save_development_connection_with_organization(
                            id,
                            form,
                            password,
                            organization,
                        )
                        .await
                    {
                        Ok(connection) => {
                            let id = connection.id;
                            (Some(id), Ok(None))
                        }
                        Err(error) => (None, Err(error)),
                    }
                })
            }
            Kind::Credentials(settings) => {
                let state = settings.state;
                Box::pin(async move {
                    let result = if matches!(action, FormAction::ConfirmReset) {
                        backend.reset_development_credentials(true).await
                    } else if matches!(action, FormAction::Recover) {
                        backend.recover_development_credentials().await
                    } else if state == DevelopmentCredentialState::NeedsUnlock {
                        backend.unlock_development_credentials(password).await
                    } else if state == DevelopmentCredentialState::NeedsOnboarding {
                        backend
                            .configure_development_credentials(
                                mode,
                                (!password.is_empty()).then_some(password),
                            )
                            .await
                    } else {
                        backend
                            .change_development_credentials(
                                mode,
                                (!password.is_empty()).then_some(password),
                                true,
                            )
                            .await
                    };
                    (None, result.map(|_| None))
                })
            }
            Kind::Delete(id) => {
                let id = id.clone();
                Box::pin(async move {
                    (
                        None,
                        backend
                            .delete_development_connection(id)
                            .await
                            .map(|_| None),
                    )
                })
            }
            _ => return,
        };
        self.busy = true;
        for field in &self.fields {
            field
                .editor
                .update(cx, |editor, _| editor.set_read_only(true));
        }
        self.message = None;
        let work = self.host.runtime.spawn(job);
        self.task = Some(cx.spawn(async move |this, cx| {
            let (saved_id, result) = work
                .await
                .unwrap_or((None, Err("Operation could not finish".into())));
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                for field in &this.fields {
                    field
                        .editor
                        .update(cx, |editor, _| editor.set_read_only(false));
                }
                if let (Some(saved), Kind::Connection { id }) = (saved_id, &mut this.kind) {
                    *id = Some(saved);
                }
                match result {
                    Ok(None) => cx.emit(FormEvent::Saved),
                    Ok(Some(message)) => this.message = Some(message),
                    Err(error) => this.message = Some(error),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
    fn button(
        &mut self,
        label: impl Into<SharedString>,
        action: FormAction,
        selected: bool,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let label = label.into();
        // A changed toggle label must not replace its element or focus handle.
        let key = match action {
            FormAction::ReadOnly => "Read-only".to_owned(),
            FormAction::Favorite => "Favorite".to_owned(),
            _ => label.to_string(),
        };
        let focus = self
            .controls
            .entry(key.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        self.visible_controls.push(focus.clone());
        let enabled = !self.busy
            || (matches!(action, FormAction::Cancel)
                && self
                    .diagnosis
                    .as_ref()
                    .is_some_and(diagnosis::State::running));
        let choice = matches!(
            action,
            FormAction::Mode(_)
                | FormAction::Environment(_)
                | FormAction::Safe(_)
                | FormAction::Tls(_)
        );
        let toggle = matches!(action, FormAction::ReadOnly | FormAction::Favorite);
        let weak = cx.weak_entity();
        div()
            .id(SharedString::from(key))
            .role(if choice {
                Role::RadioButton
            } else if toggle {
                Role::CheckBox
            } else {
                Role::Button
            })
            .aria_label(label.clone())
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(if selected {
                rgb(0xffffff)
            } else {
                rgb(0x333333)
            })
            .text_color(if enabled {
                rgb(0xffffff)
            } else {
                rgb(0x888888)
            })
            .focus(|style| style.bg(rgb(0x202020)))
            .a11y_synthetic_children(move |builder| {
                if choice || toggle {
                    builder.parent_node().set_toggled(Toggled::from(selected));
                }
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
            .on_a11y_action(Action::Click, move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
            })
            .child(label)
            .into_any_element()
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The focused editor owns composition keys. In particular Escape must
        // cancel marked input before it can dismiss the connection form.
        if self.fields.iter().any(|field| {
            field.editor.focus_handle(cx).is_focused(window)
                && field.editor.update(cx, |editor, cx| {
                    gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                })
        }) {
            return;
        }
        if event.keystroke.key == "escape" {
            self.activate(FormAction::Cancel, window, cx);
            cx.stop_propagation();
        }
        if event.keystroke.key == "tab" {
            let handles: Vec<_> = self
                .fields
                .iter()
                .map(|field| field.editor.focus_handle(cx))
                .chain(self.visible_controls.iter().cloned())
                .collect();
            if !handles.is_empty() {
                let current = handles.iter().position(|handle| handle.is_focused(window));
                let next = if event.keystroke.modifiers.shift {
                    current.map_or(handles.len() - 1, |i| {
                        (i + handles.len() - 1) % handles.len()
                    })
                } else {
                    current.map_or(0, |i| (i + 1) % handles.len())
                };
                window.focus(&handles[next], cx);
            }
            cx.stop_propagation();
        }
    }
}
fn number(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}
impl Render for Form {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_controls.clear();
        let title = match self.kind {
            Kind::Credentials(_) => "Credential storage",
            Kind::Connection { .. } => "PostgreSQL connection",
            Kind::Rename => "Rename query",
            Kind::OpenTable => "Open table",
            Kind::Delete(_) => "Delete connection",
            Kind::Discard => "Unsaved drafts",
            Kind::ResetWorkspace => "Reset saved drafts",
        };
        let mut content = div().flex().flex_col().gap_2();
        if matches!(self.kind, Kind::Connection { .. }) {
            let mut fields = div().flex().flex_wrap().gap_x_4().gap_y_2();
            for field in &self.fields {
                fields = fields.child(
                    div()
                        .w(px(430.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(170.)).child(field.label))
                        .child(
                            div()
                                .flex_1()
                                .h(px(26.))
                                .border_b_1()
                                .border_color(rgb(0x444444))
                                .child(field.accessible.clone()),
                        ),
                );
            }
            content = content
                .child(fields)
                .child(div().child("Blank password keeps the saved password."));
            let mut policy = div().flex().flex_wrap().gap_2().child("Environment");
            for (label, value) in [
                ("Development", DevelopmentEnvironment::Development),
                ("Test", DevelopmentEnvironment::Test),
                ("Staging", DevelopmentEnvironment::Staging),
                ("Production", DevelopmentEnvironment::Production),
            ] {
                policy = policy.child(self.button(
                    label,
                    FormAction::Environment(value),
                    self.environment == value,
                    cx,
                ));
            }
            content = content.child(policy);
            let mut policy = div().flex().flex_wrap().gap_2().child("Safe Mode");
            for (label, value) in [
                ("Inherit", DevelopmentSafeMode::Inherit),
                ("Disabled", DevelopmentSafeMode::Disabled),
                ("Protected", DevelopmentSafeMode::Protected),
                ("Strict", DevelopmentSafeMode::Strict),
            ] {
                policy = policy.child(self.button(
                    label,
                    FormAction::Safe(value),
                    self.safe == value,
                    cx,
                ));
            }
            policy = policy
                .child(self.button(
                    if self.read_only {
                        "Read-only: on"
                    } else {
                        "Read-only: off"
                    },
                    FormAction::ReadOnly,
                    self.read_only,
                    cx,
                ))
                .child(self.button(
                    if self.favorite {
                        "Favorite: yes"
                    } else {
                        "Favorite: no"
                    },
                    FormAction::Favorite,
                    self.favorite,
                    cx,
                ));
            content = content.child(policy);
            let mut tls = div().flex().flex_wrap().gap_2().child("TLS");
            for (label, value) in [
                ("Disable TLS", DevelopmentTlsMode::Disable),
                ("Prefer TLS", DevelopmentTlsMode::Prefer),
                ("Require TLS", DevelopmentTlsMode::Require),
                ("Verify CA", DevelopmentTlsMode::VerifyCa),
                ("Verify full", DevelopmentTlsMode::VerifyFull),
            ] {
                tls = tls.child(self.button(label, FormAction::Tls(value), self.tls == value, cx));
            }
            content = content.child(tls).child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "Choose root certificate",
                        FormAction::Pick("root-cert"),
                        false,
                        cx,
                    ))
                    .child(self.button(
                        "Choose client certificate",
                        FormAction::Pick("client-cert"),
                        false,
                        cx,
                    ))
                    .child(self.button(
                        "Choose client key",
                        FormAction::Pick("client-key"),
                        false,
                        cx,
                    )),
            );
        } else if let Kind::Credentials(settings) = &self.kind {
            let state = settings.state;
            if state == DevelopmentCredentialState::NeedsRecovery {
                content=content.child("An interrupted credential change needs recovery. Stored data is preserved.").child(self.button("Recover credentials",FormAction::Recover,false,cx));
            } else {
                if state != DevelopmentCredentialState::NeedsUnlock {
                    content = content.child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                "Plain SQLite",
                                FormAction::Mode(DevelopmentStorageMode::PlainSqlite),
                                self.mode == DevelopmentStorageMode::PlainSqlite,
                                cx,
                            ))
                            .child(self.button(
                                "Encrypted SQLite",
                                FormAction::Mode(DevelopmentStorageMode::EncryptedSqlite),
                                self.mode == DevelopmentStorageMode::EncryptedSqlite,
                                cx,
                            ))
                            .child(self.button(
                                "Keychain",
                                FormAction::Mode(DevelopmentStorageMode::Keychain),
                                self.mode == DevelopmentStorageMode::Keychain,
                                cx,
                            )),
                    );
                }
                if let Some(field) = self.fields.first() {
                    content = content.child(
                        div().flex().gap_2().child(field.label).child(
                            div()
                                .w(px(320.))
                                .h(px(28.))
                                .border_b_1()
                                .border_color(rgb(0x444444))
                                .child(field.accessible.clone()),
                        ),
                    );
                }
                content = content.child("Credential encryption does not encrypt SQL drafts.");
                if state != DevelopmentCredentialState::NeedsOnboarding {
                    content = content.child(self.button(
                        "Reset saved passwords",
                        FormAction::Reset,
                        false,
                        cx,
                    ));
                }
            }
        } else {
            for field in &self.fields {
                content = content.child(
                    div()
                        .flex()
                        .gap_2()
                        .child(field.label)
                        .child(div().w(px(400.)).h(px(28.)).child(field.accessible.clone())),
                );
            }
        }
        if let Some(message) = &self.message {
            content = content.child(
                div()
                    .id("form-message")
                    .role(Role::Alert)
                    .aria_label(message.clone())
                    .a11y_synthetic_children(|builder| {
                        builder.parent_node().set_live(Live::Polite);
                    })
                    .child(message.clone()),
            );
        }
        if self.confirm_reset {
            content = content
                .child("Reset removes saved passwords. Connections and SQL drafts are kept.")
                .child(self.button("Confirm password loss", FormAction::ConfirmReset, false, cx));
        }
        if let Some(view) = self.diagnosis.as_ref().and_then(diagnosis::State::view) {
            content = content.child(view);
        }
        let mut buttons = div().flex().gap_3().pt_3();
        let action = match &self.kind {
            Kind::OpenTable => Some(("Open", FormAction::Submit)),
            Kind::Delete(_) => Some(("Delete connection", FormAction::Delete)),
            Kind::Discard => Some(("Discard unsaved drafts and close", FormAction::Discard)),
            Kind::ResetWorkspace => Some(("Reset saved drafts", FormAction::Discard)),
            Kind::Credentials(settings)
                if settings.state == DevelopmentCredentialState::NeedsRecovery =>
            {
                None
            }
            Kind::Credentials(settings)
                if settings.state == DevelopmentCredentialState::NeedsUnlock =>
            {
                Some(("Unlock", FormAction::Submit))
            }
            _ => Some(("Save", FormAction::Submit)),
        };
        if let Some((label, action)) = action {
            buttons = buttons.child(self.button(label, action, false, cx));
        }
        if matches!(self.kind, Kind::Connection { id: None }) {
            buttons = buttons.child(self.button(
                "Import URI from clipboard",
                FormAction::ImportUri,
                false,
                cx,
            ));
        }
        if matches!(self.kind, Kind::Connection { .. }) {
            buttons = buttons.child(self.button("Test connection", FormAction::Test, false, cx));
        }
        buttons = buttons.child(self.button("Cancel", FormAction::Cancel, false, cx));
        if self.busy {
            buttons = buttons.child("Working…");
        }
        div()
            .id("native-form")
            .role(Role::Dialog)
            .aria_label(title)
            .bg(rgb(0))
            .text_color(rgb(0xffffff))
            .text_sm()
            .p_4()
            .size_full()
            .overflow_y_scroll()
            .capture_key_down(cx.listener(Self::key))
            .child(div().text_lg().mb_3().child(title))
            .child(content)
            .child(buttons)
    }
}

/// Defaults describe the selected profile capability; choosing a form never
/// saves credentials or opens a connection.
fn connection_defaults(kind: Option<NativeProfileKind>) -> DevelopmentPostgresConnection {
    let general = kind == Some(NativeProfileKind::GeneralPostgres);
    DevelopmentPostgresConnection {
        name: "Local PostgreSQL".into(),
        host: "127.0.0.1".into(),
        port: if general { 5432 } else { 15432 },
        database: if general { "postgres" } else { "dbunk_demo" }.into(),
        user: if general { "postgres" } else { "dbunk" }.into(),
        environment: DevelopmentEnvironment::Development,
        safe_mode: DevelopmentSafeMode::Protected,
        read_only: false,
        tls: DevelopmentTlsOptions {
            mode: if general {
                DevelopmentTlsMode::Prefer
            } else {
                DevelopmentTlsMode::Disable
            },
            ..Default::default()
        },
        driver_options: Default::default(),
    }
}

#[cfg(test)]
mod defaults_tests {
    use super::*;

    #[test]
    fn general_defaults_do_not_embed_the_fixture_database_or_port() {
        let general = connection_defaults(Some(NativeProfileKind::GeneralPostgres));
        assert_eq!(general.tls.mode, DevelopmentTlsMode::Prefer);
        assert_eq!(
            (
                general.port,
                general.database.as_str(),
                general.user.as_str()
            ),
            (5432, "postgres", "postgres")
        );
        for kind in [None, Some(NativeProfileKind::OwnedFixtures)] {
            let fixture = connection_defaults(kind);
            assert_eq!(fixture.tls.mode, DevelopmentTlsMode::Disable);
            assert_eq!(
                (
                    fixture.port,
                    fixture.database.as_str(),
                    fixture.user.as_str()
                ),
                (15432, "dbunk_demo", "dbunk")
            );
        }
    }
}

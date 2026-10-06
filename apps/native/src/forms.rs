//! Native forms share one editing model, secure AX fields and deterministic
//! keyboard return. Stored passwords are never loaded into an editor.
use crate::{
    accessible_editor::{AccessibleEditor, fresh_editor},
    controller::Host,
};
use dbunk_lib::backend::*;
use editor::Editor;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, PathPromptOptions,
    Role, SharedString, Task, Window,
    accesskit::{Action, Live, Toggled},
    div,
    prelude::*,
    px,
};
use std::{collections::HashMap, sync::Arc};

mod bastions;
mod diagnosis;
mod engine;
mod engine_view;
mod render;
mod tunnel;
mod uri;
mod validation;

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
    Bastion(bastions::Action),
    Tunnel,
    TunnelVia(usize),
    Engine(engine::Engine),
    EngineToggle(engine::Toggle),
    Acknowledge,
}
/// How a form message reads: errors shake and take the danger colour.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Tone {
    Info,
    Success,
    Error,
}
enum Kind {
    Credentials(DevelopmentSettings),
    Connection { id: Option<String> },
    Rename,
    OpenTable,
    Delete(String),
    Discard,
    ResetWorkspace,
    Bastions(Box<bastions::State>),
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
    tone: Tone,
    /// Bumped by every new message so a repeated error still shakes.
    message_seq: u64,
    /// Body text of a confirmation page (delete, discard, reset).
    prompt: Option<String>,
    acknowledged: bool,
    /// Inline field errors show after the first submit or test.
    attempted: bool,
    task: Option<Task<()>>,
    diagnosis: Option<diagnosis::State>,
    tunnel: Option<tunnel::State>,
    engine: engine::Engine,
    toggles: engine::Toggles,
}
impl EventEmitter<FormEvent> for Form {}
impl Form {
    fn base(host: Arc<Host>, kind: Kind, cx: &Context<Self>) -> Self {
        Self {
            host,
            kind,
            fields: Vec::new(),
            controls: HashMap::from([
                ("Cancel".into(), cx.focus_handle()),
                ("Submit".into(), cx.focus_handle()),
                ("Recover credentials".into(), cx.focus_handle()),
            ]),
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
            tone: Tone::Info,
            message_seq: 0,
            prompt: None,
            acknowledged: false,
            attempted: false,
            task: None,
            diagnosis: None,
            tunnel: None,
            engine: engine::Engine::Postgres,
            toggles: engine::Toggles::default(),
        }
    }
    fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.message = Some(text.into());
        self.tone = tone;
        self.message_seq = self.message_seq.wrapping_add(1);
    }
    fn fail(&mut self, text: impl Into<String>) {
        self.say(text, Tone::Error);
    }
    fn note(&mut self, text: impl Into<String>) {
        self.say(text, Tone::Info);
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
        let editor = self.field_editor(value, secret, window, cx);
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, secret, cx));
        self.fields.push(Field {
            key,
            label,
            editor,
            accessible,
        });
    }
    /// Initial and URI-imported fields share construction, so every editor
    /// invalidates a stale diagnosis when edited.
    fn field_editor(
        &mut self,
        value: String,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let editor = cx.new(|cx| {
            let mut editor = fresh_editor(value, editor::EditorMode::SingleLine, window, cx);
            editor.set_masked(secret, cx);
            editor
        });
        cx.subscribe(&editor, |this, _, event, cx| {
            if !matches!(event, editor::EditorEvent::BufferEdited) {
                return;
            }
            if let Some(state) = &mut this.diagnosis {
                state.invalidate();
                this.message = None;
            }
            // Inline errors follow the text once they are showing.
            if this.attempted || this.diagnosis.is_some() {
                cx.notify();
            }
        })
        .detach();
        editor
    }
    pub fn credentials(
        host: Arc<Host>,
        settings: DevelopmentSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Encrypted SQLite is the recommended first choice, as before.
        let mode = settings
            .mode
            .unwrap_or(DevelopmentStorageMode::EncryptedSqlite);
        let state = settings.state;
        let mut form = Self::base(host, Kind::Credentials(settings), cx);
        form.mode = mode;
        if state != DevelopmentCredentialState::NeedsRecovery {
            form.field(
                "password",
                if state == DevelopmentCredentialState::NeedsUnlock {
                    "Credential password"
                } else {
                    "New credential password"
                },
                String::new(),
                true,
                window,
                cx,
            );
        }
        if matches!(
            state,
            DevelopmentCredentialState::NeedsOnboarding | DevelopmentCredentialState::Ready
        ) {
            form.field(
                "confirm",
                "Confirm password",
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
        // Plan 031: non-PostgreSQL records edit through their own field set.
        let settings = connection
            .as_ref()
            .and_then(|connection| connection.settings.clone())
            .filter(|settings| engine::Engine::of(settings) != engine::Engine::Postgres);
        let mut form = Self::base(
            host,
            Kind::Connection {
                id: connection.as_ref().map(|connection| connection.id.clone()),
            },
            cx,
        );
        form.diagnosis = Some(diagnosis::State::new(retained));
        if form.host.backend.native_profile_kind() == Some(NativeProfileKind::GeneralPostgres) {
            form.tunnel = Some(tunnel::State::new(match &settings {
                Some(settings) => engine::stored_tunnel(settings),
                None => data.ssh_tunnel.clone(),
            }));
        }
        form.environment = data.environment;
        form.safe = data.safe_mode;
        form.tls = data.tls.mode;
        form.read_only = data.read_only;
        let mut engine_values = Vec::new();
        if let Some(settings) = &settings {
            let policy = engine::Policy::of(settings);
            form.engine = engine::Engine::of(settings);
            form.toggles = engine::Toggles::from_settings(settings);
            form.environment = policy.environment;
            form.safe = policy.safe_mode;
            form.read_only = policy.read_only;
            engine_values = engine::initial_values(settings);
        }
        form.favorite = connection
            .as_ref()
            .is_some_and(|connection| connection.organization.is_favorite);
        for (key, label, value) in [
            ("name", "Name", data.name),
            ("path", "Database file", String::new()),
            ("host", "Host", data.host),
            ("port", "Port", data.port.to_string()),
            ("db-number", "Database number", "0".into()),
            ("database", "Database", data.database),
            ("url-path", "URL path", String::new()),
            ("user", "User", data.user),
            ("password", "Database password", String::new()),
            (
                "project",
                "Project",
                connection
                    .as_ref()
                    .map(|c| c.organization.project.clone())
                    .unwrap_or_default(),
            ),
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
            let value = engine_values
                .iter()
                .find(|(engine_key, _)| *engine_key == key)
                .map_or(value, |(_, engine_value)| engine_value.clone());
            form.field(key, label, value, key == "password", window, cx);
        }
        if let Some(initial) = form
            .tunnel
            .as_ref()
            .map(|tunnel| tunnel::FIELDS.map(|(key, label)| (key, label, tunnel.initial(key))))
        {
            for (key, label, value) in initial {
                form.field(key, label, value, false, window, cx);
            }
            form.load_tunnel_choices(window, cx);
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
        form.prompt = Some(format!(
            "Delete {}? Saved query drafts will be kept, disconnected.",
            connection.name
        ));
        form.focus(window, cx);
        form
    }
    pub fn discard_drafts(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::Discard, cx);
        form.prompt =
            Some("Discard unsaved drafts and close? The last saved workspace will be kept.".into());
        form.focus(window, cx);
        form
    }
    pub fn reset_workspace(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::ResetWorkspace, cx);
        form.prompt = Some("Reset saved query drafts? Connections are kept and the workspace stays open. Export the saved JSON first if you need to preserve it.".into());
        form.focus(window, cx);
        form
    }
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(field) = self
            .fields
            .iter()
            .find(|field| self.field_visible(field.key))
        {
            window.focus(&field.editor.focus_handle(cx), cx);
        } else {
            // Gates have no Cancel; land on the action that moves them on.
            let key = match &self.kind {
                Kind::Credentials(settings)
                    if settings.state == DevelopmentCredentialState::NeedsRecovery =>
                {
                    "Recover credentials"
                }
                _ if !self.dismissible() => "Submit",
                _ => "Cancel",
            };
            if let Some(handle) = self.controls.get(key) {
                window.focus(handle, cx);
            }
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
            ssh_tunnel: self.tunnel_input(cx)?,
        })
    }
    /// The selected engine's record; PostgreSQL keeps its dedicated input.
    fn engine_input(&self, cx: &App) -> Result<DevelopmentEngineConnection, String> {
        if self.engine == engine::Engine::Postgres {
            return self
                .connection_input(cx)
                .map(DevelopmentEngineConnection::PostgreSQL);
        }
        let ssh_tunnel = if self.engine.tunnels() {
            self.tunnel_input(cx)?
        } else {
            None
        };
        engine::input(
            self.engine,
            |key| self.value(key, cx),
            engine::Policy {
                environment: self.environment,
                safe_mode: self.safe,
                read_only: self.read_only,
            },
            self.toggles,
            ssh_tunnel,
        )
    }
    /// Only a new connection in a general profile may change engine.
    fn engine_selectable(&self) -> bool {
        matches!(self.kind, Kind::Connection { id: None })
            && self.host.backend.native_profile_kind() == Some(NativeProfileKind::GeneralPostgres)
    }
    fn select_engine(&mut self, to: engine::Engine, window: &mut Window, cx: &mut Context<Self>) {
        if !self.engine_selectable() || to == self.engine {
            return;
        }
        let changes = engine::switch_defaults(self.engine, to, |key| self.value(key, cx));
        for (key, value) in changes {
            if let Some(field) = self.fields.iter().find(|field| field.key == key) {
                field
                    .editor
                    .update(cx, |editor, cx| editor.set_text(value, window, cx));
            }
        }
        self.engine = to;
    }
    /// The credential gate cannot be dismissed until storage is ready: the
    /// workspace behind it has no usable credentials.
    fn dismissible(&self) -> bool {
        match &self.kind {
            Kind::Credentials(settings) => settings.state == DevelopmentCredentialState::Ready,
            _ => true,
        }
    }
    /// Inline errors for the visible fields of this form.
    fn field_errors(&self, cx: &App) -> validation::Errors {
        match &self.kind {
            Kind::Connection { .. } => {
                validation::connection(self.engine, |key| self.value(key, cx))
            }
            Kind::Credentials(settings) => validation::credentials(
                settings.state,
                self.mode,
                &self.value("password", cx),
                &self.value("confirm", cx),
                self.acknowledged,
            ),
            _ => Vec::new(),
        }
    }
    /// A summary error when `action` must not run with the current input.
    fn blocking_errors(&self, action: FormAction, cx: &App) -> Option<String> {
        if matches!(
            action,
            FormAction::ConfirmReset | FormAction::Recover | FormAction::Delete
        ) {
            return None;
        }
        let errors = self.field_errors(cx);
        match errors.len() {
            0 => None,
            1 => Some(errors[0].1.clone()),
            count => Some(format!("Fix the {count} highlighted fields")),
        }
    }
    fn field_visible(&self, key: &str) -> bool {
        match &self.kind {
            Kind::Bastions(state) => bastions::field_visible(state.auth(), key),
            Kind::Credentials(settings) => {
                !self.confirm_reset
                    && (settings.state == DevelopmentCredentialState::NeedsUnlock
                        || self.mode == DevelopmentStorageMode::EncryptedSqlite)
            }
            Kind::Connection { .. } if !self.engine.shows(key) => false,
            _ if key.starts_with("tunnel-") => {
                self.tunnel.as_ref().is_some_and(|tunnel| tunnel.enabled)
            }
            _ => true,
        }
    }
    fn activate(&mut self, action: FormAction, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            if matches!(action, FormAction::Cancel)
                && let Some(state) = &self.diagnosis
                && state.running()
            {
                state.cancel();
                self.note("Cancelling connection test…");
                cx.notify();
            }
            return;
        }
        if matches!(action, FormAction::Test) {
            if let Some(errors) = self.blocking_errors(action, cx) {
                self.attempted = true;
                self.fail(errors);
                cx.notify();
                return;
            }
            self.test_connection(cx);
            return;
        }
        if let Kind::Bastions(state) = &self.kind {
            match action {
                FormAction::Bastion(action) => self.activate_bastion(action, window, cx),
                FormAction::Submit if state.editing() => self.save_bastion(window, cx),
                FormAction::Cancel => cx.emit(if self.bastions_changed() {
                    FormEvent::Saved
                } else {
                    FormEvent::Cancelled
                }),
                _ => {}
            }
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
                if self.confirm_reset {
                    self.confirm_reset = false;
                    self.message = None;
                    cx.notify();
                } else if self.dismissible() {
                    cx.emit(FormEvent::Cancelled);
                }
                return;
            }
            FormAction::Acknowledge => {
                self.acknowledged = !self.acknowledged;
                cx.notify();
                return;
            }
            FormAction::Discard => {
                cx.emit(FormEvent::ConfirmedDiscard);
                return;
            }
            FormAction::Mode(mode) => {
                if self.mode != mode {
                    self.acknowledged = false;
                }
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
            FormAction::Tunnel => {
                if let Some(tunnel) = self.tunnel.as_mut() {
                    tunnel.enabled = !tunnel.enabled;
                }
                cx.notify();
                return;
            }
            FormAction::Engine(value) => {
                self.select_engine(value, window, cx);
                cx.notify();
                return;
            }
            FormAction::EngineToggle(toggle) => {
                self.toggles.flip(toggle);
                cx.notify();
                return;
            }
            FormAction::TunnelVia(index) => {
                if let Some(tunnel) = self.tunnel.as_mut()
                    && let Some((id, _)) = tunnel.choices.get(index)
                {
                    tunnel.bastion = Some(id.clone());
                }
                cx.notify();
                return;
            }
            FormAction::Reset => {
                self.confirm_reset = true;
                self.message = None;
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
                self.fail("Enter a query name");
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
                self.fail("Enter a schema and table name (at most 256 bytes each)");
                cx.notify();
            } else {
                cx.emit(FormEvent::OpenTable { schema, table });
            }
            return;
        }
        if let Some(errors) = self.blocking_errors(action, cx) {
            self.attempted = true;
            self.fail(errors);
            cx.notify();
            return;
        }
        let backend = self.host.backend.clone();
        let password = self.value("password", cx);
        let mode = self.mode;
        // Only Encrypted SQLite takes a password; never send a stray one.
        let storage_password = (mode == DevelopmentStorageMode::EncryptedSqlite
            && !password.is_empty())
        .then(|| password.clone());
        let job: FormJob = match &self.kind {
            Kind::Connection { id } => {
                let form = match self.engine_input(cx) {
                    Ok(form) => form,
                    Err(error) => {
                        self.fail(error);
                        cx.notify();
                        return;
                    }
                };
                let id = id.clone();
                let organization = DevelopmentConnectionOrganization {
                    folder: self.value("folder", cx),
                    is_favorite: self.favorite,
                    color: self.value("color", cx),
                    project: self.value("project", cx),
                };
                Box::pin(async move {
                    match backend
                        .save_development_engine_connection(id, form, password, organization)
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
                            .configure_development_credentials(mode, storage_password)
                            .await
                    } else {
                        backend
                            .change_development_credentials(mode, storage_password, true)
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
                    Ok(Some(message)) => this.note(message),
                    Err(error) => this.fail(error),
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
            FormAction::Cancel => "Cancel".to_owned(),
            FormAction::Submit => "Submit".to_owned(),
            FormAction::Acknowledge => "Acknowledge".to_owned(),
            FormAction::Tunnel => "SSH tunnel".to_owned(),
            FormAction::TunnelVia(index) => format!("tunnel-via-{index}"),
            FormAction::Engine(engine) => format!("engine-{}", engine.label()),
            FormAction::EngineToggle(toggle) => format!("engine-toggle-{toggle:?}"),
            FormAction::Mode(mode) => format!("mode-{mode:?}"),
            FormAction::Bastion(bastions::Action::ClearPassphrase) => "Clear passphrase".to_owned(),
            FormAction::Bastion(bastions::Action::Row(op, index)) => {
                format!("bastion-{op:?}-{index}")
            }
            FormAction::Bastion(action) => format!("bastion-{action:?}"),
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
                | FormAction::TunnelVia(_)
                | FormAction::Engine(_)
                | FormAction::Bastion(bastions::Action::Auth(_))
        );
        let toggle = matches!(
            action,
            FormAction::ReadOnly
                | FormAction::Favorite
                | FormAction::Tunnel
                | FormAction::Acknowledge
                | FormAction::EngineToggle(_)
                | FormAction::Bastion(bastions::Action::ClearPassphrase)
        );
        let weak = cx.weak_entity();
        let element = if choice {
            render::chip(key, label, selected, enabled, chip_dot(action))
        } else if toggle {
            render::toggle_row(key, label, selected, enabled)
        } else {
            crate::ui::button(key, label, variant(action), enabled)
        };
        element
            .role(if choice {
                Role::RadioButton
            } else if toggle {
                Role::CheckBox
            } else {
                Role::Button
            })
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if choice || toggle {
                    builder.parent_node().set_toggled(Toggled::from(selected));
                }
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            .when(enabled, |element| {
                element.on_click(
                    cx.listener(move |this, _, window, cx| this.activate(action, window, cx)),
                )
            })
            .on_a11y_action(Action::Click, move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
            })
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
                .filter(|field| self.field_visible(field.key))
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
/// Button weight follows what the action does.
fn variant(action: FormAction) -> crate::ui::Variant {
    use crate::ui::Variant;
    match action {
        FormAction::Submit => Variant::Primary,
        FormAction::Delete | FormAction::Discard | FormAction::ConfirmReset => Variant::Danger,
        FormAction::Cancel | FormAction::Reset | FormAction::Bastion(bastions::Action::Back) => {
            Variant::Ghost
        }
        _ => Variant::Secondary,
    }
}
/// Environment choices carry their signal colour.
fn chip_dot(action: FormAction) -> Option<u32> {
    match action {
        FormAction::Environment(environment) => Some(crate::style::env(Some(environment))),
        _ => None,
    }
}
fn number(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
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
        ssh_tunnel: None,
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

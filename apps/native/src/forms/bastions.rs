//! C08.b/C08.c Bastion Server settings. Secrets are write-only masked fields:
//! stored values are never loaded, blank fields keep them, and the editors are
//! dropped when editing ends. Host-key trust changes only by explicit review.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RowOp {
    Edit,
    Delete,
    Test,
    Trust,
    Reset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    New,
    Row(RowOp, usize),
    Auth(DevelopmentBastionAuth),
    ClearPassphrase,
    Back,
    ConfirmDelete,
}

enum Outcome {
    Listed(Vec<DevelopmentBastion>),
    Changed(Vec<DevelopmentBastion>, &'static str),
    Review(String, Vec<DevelopmentBastionReference>),
    Tested(String, DevelopmentBastionTest),
}

type Job = futures_util::future::BoxFuture<'static, Result<Outcome, String>>;

pub(super) struct State {
    list: Vec<DevelopmentBastion>,
    /// `Some(None)` creates; `Some(Some(id))` edits.
    editing: Option<Option<String>>,
    auth: DevelopmentBastionAuth,
    clear_passphrase: bool,
    review: Option<(String, Vec<DevelopmentBastionReference>)>,
    tests: HashMap<String, DevelopmentBastionTest>,
    pub(super) changed: bool,
    loaded: bool,
}

impl State {
    fn new() -> Self {
        Self {
            list: Vec::new(),
            editing: None,
            auth: DevelopmentBastionAuth::Password,
            clear_passphrase: false,
            review: None,
            tests: HashMap::new(),
            changed: false,
            loaded: false,
        }
    }

    pub(super) fn editing(&self) -> bool {
        self.editing.is_some()
    }

    pub(super) fn auth(&self) -> DevelopmentBastionAuth {
        self.auth
    }
}

/// Field keys shown for an authentication method. Hidden fields are skipped
/// by rendering and Tab order alike.
pub(super) fn field_visible(auth: DevelopmentBastionAuth, key: &str) -> bool {
    match key {
        "bastion-password" => auth == DevelopmentBastionAuth::Password,
        "bastion-key-path" => auth == DevelopmentBastionAuth::PrivateKeyPath,
        "bastion-key-content" => auth == DevelopmentBastionAuth::PrivateKeyContent,
        "bastion-passphrase" => auth != DevelopmentBastionAuth::Password,
        _ => true,
    }
}

fn auth_label(auth: DevelopmentBastionAuth) -> &'static str {
    match auth {
        DevelopmentBastionAuth::Password => "Password",
        DevelopmentBastionAuth::PrivateKeyPath => "Private key file",
        DevelopmentBastionAuth::PrivateKeyContent => "Private key content",
    }
}

/// One line describing a Test, never containing a secret.
pub(super) fn test_summary(test: &DevelopmentBastionTest) -> String {
    let key = match &test.host_key {
        DevelopmentHostKeyStatus::Trusted => {
            format!("Host key {} is trusted", test.observed_fingerprint)
        }
        DevelopmentHostKeyStatus::Unknown => format!(
            "Host key {} is not trusted yet. Compare it with the server's published fingerprint before trusting it",
            test.observed_fingerprint
        ),
        DevelopmentHostKeyStatus::Changed { trusted } => format!(
            "Host key CHANGED: trusted {trusted}, server now presents {}. Connections are refused until you review and replace it",
            test.observed_fingerprint
        ),
    };
    let auth = match &test.authentication {
        DevelopmentBastionAuthentication::NotAttempted => "credentials were not sent".to_owned(),
        DevelopmentBastionAuthentication::Authenticated => {
            format!("authenticated in {} ms", test.latency_ms)
        }
        DevelopmentBastionAuthentication::Failed { message } => {
            format!("authentication failed: {message}")
        }
    };
    format!("{key}; {auth}.")
}

/// Explicit review wording for a trust action, or none when already trusted.
pub(super) fn trust_label(name: &str, test: &DevelopmentBastionTest) -> Option<String> {
    match test.host_key {
        DevelopmentHostKeyStatus::Trusted => None,
        DevelopmentHostKeyStatus::Unknown => {
            Some(format!("Trust {} for {name}", test.observed_fingerprint))
        }
        DevelopmentHostKeyStatus::Changed { .. } => Some(format!(
            "Replace trusted key for {name} with {}",
            test.observed_fingerprint
        )),
    }
}

impl Form {
    pub fn bastions(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut form = Self::base(host, Kind::Bastions(Box::new(State::new())), cx);
        let backend = form.host.backend.clone();
        form.run_bastion(
            Box::pin(async move { backend.development_bastions().await.map(Outcome::Listed) }),
            window,
            cx,
        );
        form.focus(window, cx);
        form
    }

    fn bastion_state(&mut self) -> Option<&mut State> {
        match &mut self.kind {
            Kind::Bastions(state) => Some(state),
            _ => None,
        }
    }

    pub(super) fn bastions_changed(&self) -> bool {
        matches!(&self.kind, Kind::Bastions(state) if state.changed)
    }

    fn run_bastion(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        for field in &self.fields {
            field
                .editor
                .update(cx, |editor, _| editor.set_read_only(true));
        }
        let work = self.host.runtime.spawn(job);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|_| Err("Operation could not finish".into()));
            let _ = this.update_in(cx, |form, window, cx| {
                form.finish_bastion(result, window, cx)
            });
        }));
        cx.notify();
    }

    fn finish_bastion(
        &mut self,
        result: Result<Outcome, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        for field in &self.fields {
            field
                .editor
                .update(cx, |editor, _| editor.set_read_only(false));
        }
        let Some(state) = self.bastion_state() else {
            return;
        };
        state.loaded = true;
        let mut leave_editor = false;
        let message = match result {
            Ok(Outcome::Listed(list)) => {
                state.list = list;
                None
            }
            Ok(Outcome::Changed(list, message)) => {
                state.list = list;
                state.changed = true;
                state.review = None;
                leave_editor = true;
                Some(message.to_owned())
            }
            Ok(Outcome::Review(id, references)) => {
                state.review = Some((id, references));
                Some("Connections using this Bastion Server changed; review again".into())
            }
            Ok(Outcome::Tested(id, test)) => {
                let summary = test_summary(&test);
                state.tests.insert(id, test);
                Some(summary)
            }
            Err(error) => Some(error),
        };
        if leave_editor && state.editing.take().is_some() {
            // Secret editors are dropped with the editor view.
            self.fields.clear();
            self.focus(window, cx);
        }
        self.message = message;
        cx.notify();
    }

    pub(super) fn activate_bastion(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let backend = self.host.backend.clone();
        let Some(state) = self.bastion_state() else {
            return;
        };
        let row = |state: &State, index: usize| state.list.get(index).cloned();
        match action {
            Action::New => {
                state.editing = Some(None);
                state.auth = DevelopmentBastionAuth::Password;
                state.clear_passphrase = false;
                state.review = None;
                self.bastion_fields(None, window, cx);
            }
            Action::Row(RowOp::Edit, index) => {
                let Some(bastion) = row(state, index) else {
                    return;
                };
                state.editing = Some(Some(bastion.id.clone()));
                state.auth = bastion.form.auth_method;
                state.clear_passphrase = false;
                state.review = None;
                self.bastion_fields(Some(&bastion), window, cx);
            }
            Action::Row(RowOp::Delete, index) => {
                let Some(bastion) = row(state, index) else {
                    return;
                };
                state.review = Some((bastion.id, bastion.references));
                self.message = None;
                cx.notify();
            }
            Action::ConfirmDelete => {
                let Some((id, references)) = state.review.clone() else {
                    return;
                };
                let reviewed = references
                    .iter()
                    .map(|reference| reference.connection_id.clone())
                    .collect();
                self.run_bastion(
                    Box::pin(async move {
                        match backend
                            .delete_development_bastion(id.clone(), reviewed)
                            .await?
                        {
                            DevelopmentBastionDelete::Deleted => Ok(Outcome::Changed(
                                backend.development_bastions().await?,
                                "Bastion Server deleted",
                            )),
                            DevelopmentBastionDelete::ReviewRequired { references } => {
                                Ok(Outcome::Review(id, references))
                            }
                        }
                    }),
                    window,
                    cx,
                );
            }
            Action::Row(RowOp::Test, index) => {
                let Some(bastion) = row(state, index) else {
                    return;
                };
                self.message = Some(format!("Testing {}…", bastion.form.name));
                self.run_bastion(
                    Box::pin(async move {
                        let test = backend.test_development_bastion(bastion.id.clone()).await?;
                        Ok(Outcome::Tested(bastion.id, test))
                    }),
                    window,
                    cx,
                );
            }
            Action::Row(RowOp::Trust, index) => {
                let Some(bastion) = row(state, index) else {
                    return;
                };
                let Some(test) = state.tests.remove(&bastion.id) else {
                    return;
                };
                // The reviewed baseline is the key trusted when the Test ran.
                let expected = match &test.host_key {
                    DevelopmentHostKeyStatus::Changed { trusted } => Some(trusted.clone()),
                    _ => bastion.host_key_fingerprint.clone(),
                };
                self.run_bastion(
                    Box::pin(async move {
                        backend
                            .trust_development_bastion_host_key(
                                bastion.id,
                                expected,
                                test.observed_fingerprint,
                            )
                            .await?;
                        Ok(Outcome::Changed(
                            backend.development_bastions().await?,
                            "Host key trusted",
                        ))
                    }),
                    window,
                    cx,
                );
            }
            Action::Row(RowOp::Reset, index) => {
                let Some(bastion) = row(state, index) else {
                    return;
                };
                state.tests.remove(&bastion.id);
                self.run_bastion(
                    Box::pin(async move {
                        backend
                            .reset_development_bastion_host_key(bastion.id)
                            .await?;
                        Ok(Outcome::Changed(
                            backend.development_bastions().await?,
                            "Host-key trust reset; test the Bastion Server to review its key",
                        ))
                    }),
                    window,
                    cx,
                );
            }
            Action::Auth(auth) => {
                state.auth = auth;
                cx.notify();
            }
            Action::ClearPassphrase => {
                state.clear_passphrase = !state.clear_passphrase;
                cx.notify();
            }
            Action::Back => {
                state.editing = None;
                state.review = None;
                self.fields.clear();
                self.message = None;
                self.focus(window, cx);
                cx.notify();
            }
        }
    }

    pub(super) fn save_bastion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Kind::Bastions(state) = &self.kind else {
            return;
        };
        let Some(id) = state.editing.clone() else {
            return;
        };
        let auth = state.auth;
        let clear_passphrase = state.clear_passphrase;
        let port = match self.value("bastion-port", cx).trim().parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => {
                self.message = Some("Bastion port must be between 1 and 65535".into());
                cx.notify();
                return;
            }
        };
        let secret = |key: &str| {
            let value = self.value(key, cx);
            if field_visible(auth, key) && !value.is_empty() {
                DevelopmentSecretInput::Set(value)
            } else {
                DevelopmentSecretInput::Keep
            }
        };
        let secrets = DevelopmentBastionSecrets {
            password: secret("bastion-password"),
            private_key_content: secret("bastion-key-content"),
            passphrase: if clear_passphrase {
                DevelopmentSecretInput::Clear
            } else {
                secret("bastion-passphrase")
            },
        };
        let key_path = self.value("bastion-key-path", cx);
        let form = DevelopmentBastionForm {
            name: self.value("bastion-name", cx),
            host: self.value("bastion-host", cx),
            port,
            user: self.value("bastion-user", cx),
            auth_method: auth,
            private_key_path: (auth == DevelopmentBastionAuth::PrivateKeyPath
                && !key_path.trim().is_empty())
            .then_some(key_path),
        };
        let backend = self.host.backend.clone();
        self.run_bastion(
            Box::pin(async move {
                backend.save_development_bastion(id, form, secrets).await?;
                Ok(Outcome::Changed(
                    backend.development_bastions().await?,
                    "Bastion Server saved",
                ))
            }),
            window,
            cx,
        );
    }

    fn bastion_fields(
        &mut self,
        bastion: Option<&DevelopmentBastion>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.fields.clear();
        self.message = None;
        let form = bastion.map(|bastion| bastion.form.clone());
        for (key, label, value) in [
            (
                "bastion-name",
                "Bastion name",
                form.as_ref().map(|f| f.name.clone()).unwrap_or_default(),
            ),
            (
                "bastion-host",
                "SSH host",
                form.as_ref().map(|f| f.host.clone()).unwrap_or_default(),
            ),
            (
                "bastion-port",
                "SSH port",
                form.as_ref()
                    .map_or_else(|| "22".into(), |f| f.port.to_string()),
            ),
            (
                "bastion-user",
                "SSH user",
                form.as_ref().map(|f| f.user.clone()).unwrap_or_default(),
            ),
            (
                "bastion-key-path",
                "Private key file",
                form.as_ref()
                    .and_then(|f| f.private_key_path.clone())
                    .unwrap_or_default(),
            ),
        ] {
            self.field(key, label, value, false, window, cx);
        }
        self.field(
            "bastion-password",
            "SSH password",
            String::new(),
            true,
            window,
            cx,
        );
        self.multiline_secret("bastion-key-content", "Private key content", window, cx);
        self.field(
            "bastion-passphrase",
            "Key passphrase",
            String::new(),
            true,
            window,
            cx,
        );
        self.focus(window, cx);
        cx.notify();
    }

    fn multiline_secret(
        &mut self,
        key: &'static str,
        label: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| {
            let mut editor = Editor::multi_line(window, cx);
            editor.set_masked(true, cx);
            editor
        });
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, true, cx));
        self.fields.push(Field {
            key,
            label,
            editor,
            accessible,
        });
    }

    pub(super) fn bastion_view(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let Kind::Bastions(state) = &self.kind else {
            return div();
        };
        let mut content = div().flex().flex_col().gap_2();
        if let Some(id) = &state.editing {
            let auth = state.auth;
            let clear_passphrase = state.clear_passphrase;
            let existing = id
                .as_ref()
                .and_then(|id| state.list.iter().find(|bastion| &bastion.id == id))
                .cloned();
            let mut methods = div().flex().flex_wrap().gap_2().child("Authentication");
            for value in [
                DevelopmentBastionAuth::Password,
                DevelopmentBastionAuth::PrivateKeyPath,
                DevelopmentBastionAuth::PrivateKeyContent,
            ] {
                methods = methods.child(self.button(
                    auth_label(value),
                    FormAction::Bastion(Action::Auth(value)),
                    auth == value,
                    cx,
                ));
            }
            content = content.child(methods);
            let mut fields = div().flex().flex_col().gap_2();
            for field in &self.fields {
                if !field_visible(auth, field.key) {
                    continue;
                }
                let tall = field.key == "bastion-key-content";
                fields = fields.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(170.)).child(field.label))
                        .child(
                            div()
                                .w(px(420.))
                                .h(px(if tall { 96. } else { 26. }))
                                .border_b_1()
                                .border_color(rgb(0x444444))
                                .child(field.accessible.clone()),
                        ),
                );
            }
            content = content.child(fields);
            let saved = existing.map(|bastion| {
                let yes = |value: bool| if value { "saved" } else { "not saved" };
                format!(
                    "Password {}; key content {}; passphrase {}. Blank secret fields keep saved values.",
                    yes(bastion.has_password),
                    yes(bastion.has_private_key_content),
                    yes(bastion.has_passphrase)
                )
            });
            content = content.child(
                saved.unwrap_or_else(|| "Secrets are stored with your credential storage.".into()),
            );
            if auth != DevelopmentBastionAuth::Password {
                content = content.child(self.button(
                    if clear_passphrase {
                        "Clear saved passphrase: yes"
                    } else {
                        "Clear saved passphrase: no"
                    },
                    FormAction::Bastion(Action::ClearPassphrase),
                    clear_passphrase,
                    cx,
                ));
            }
            return content;
        }
        if !state.loaded {
            return content.child("Loading Bastion Servers…");
        }
        let list = state.list.clone();
        let review = state.review.clone();
        let tests = state.tests.clone();
        if list.is_empty() {
            content = content.child("No Bastion Servers yet.");
        }
        let mut rows = div()
            .id("bastion-list")
            .role(Role::List)
            .aria_label("Bastion Servers")
            .flex()
            .flex_col()
            .gap_3();
        for (index, bastion) in list.iter().enumerate() {
            let name = &bastion.form.name;
            let summary = format!(
                "{name}: {}@{}:{} · {} · host key {} · used by {} connection(s)",
                bastion.form.user,
                bastion.form.host,
                bastion.form.port,
                auth_label(bastion.form.auth_method),
                bastion
                    .host_key_fingerprint
                    .as_deref()
                    .unwrap_or("not trusted"),
                bastion.references.len()
            );
            let mut row = div()
                .id(("bastion-row", index))
                .role(Role::ListItem)
                .aria_label(summary.clone())
                .flex()
                .flex_col()
                .gap_1()
                .child(summary);
            let mut actions = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(self.button(
                    format!("Edit {name}"),
                    FormAction::Bastion(Action::Row(RowOp::Edit, index)),
                    false,
                    cx,
                ))
                .child(self.button(
                    format!("Test {name}"),
                    FormAction::Bastion(Action::Row(RowOp::Test, index)),
                    false,
                    cx,
                ));
            if bastion.host_key_fingerprint.is_some() {
                actions = actions.child(self.button(
                    format!("Reset host key for {name}"),
                    FormAction::Bastion(Action::Row(RowOp::Reset, index)),
                    false,
                    cx,
                ));
            }
            actions = actions.child(self.button(
                format!("Delete {name}"),
                FormAction::Bastion(Action::Row(RowOp::Delete, index)),
                false,
                cx,
            ));
            row = row.child(actions);
            if let Some(test) = tests.get(&bastion.id) {
                let line = test_summary(test);
                row = row.child(
                    div()
                        .id(("bastion-test", index))
                        .role(Role::Status)
                        .aria_label(line.clone())
                        .child(line),
                );
                if let Some(label) = trust_label(name, test) {
                    row = row.child(self.button(
                        label,
                        FormAction::Bastion(Action::Row(RowOp::Trust, index)),
                        false,
                        cx,
                    ));
                }
            }
            if let Some((_, references)) = review.as_ref().filter(|(id, _)| id == &bastion.id) {
                let consequence = if references.is_empty() {
                    format!("Delete {name}? Its saved SSH secrets are removed.")
                } else {
                    let names = references
                        .iter()
                        .map(|reference| reference.connection_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "Delete {name}? {} connection(s) keep a route through it and will fail to connect until edited: {names}. Their open sessions close now.",
                        references.len()
                    )
                };
                row = row
                    .child(
                        div()
                            .id(("bastion-delete-review", index))
                            .role(Role::Alert)
                            .aria_label(consequence.clone())
                            .child(consequence),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                if references.is_empty() {
                                    format!("Confirm delete {name}")
                                } else {
                                    format!(
                                        "Delete {name} and leave {} route(s) failing",
                                        references.len()
                                    )
                                },
                                FormAction::Bastion(Action::ConfirmDelete),
                                false,
                                cx,
                            ))
                            .child(self.button(
                                format!("Keep {name}"),
                                FormAction::Bastion(Action::Back),
                                false,
                                cx,
                            )),
                    );
            }
            rows = rows.child(row);
        }
        content
            .child(rows)
            .child(self.button(
                "+ Bastion Server",
                FormAction::Bastion(Action::New),
                false,
                cx,
            ))
            .child("A first-seen host key is never trusted automatically: Test a Bastion Server, compare its fingerprint, then trust it. SSH-routed TLS verifies the certificate against the database host name.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test(host_key: DevelopmentHostKeyStatus) -> DevelopmentBastionTest {
        DevelopmentBastionTest {
            observed_fingerprint: "SHA256:observed".into(),
            host_key,
            authentication: DevelopmentBastionAuthentication::NotAttempted,
            latency_ms: 3,
        }
    }

    #[test]
    fn changed_and_unknown_keys_require_explicitly_worded_review() {
        assert_eq!(
            trust_label("Edge", &test(DevelopmentHostKeyStatus::Trusted)),
            None
        );
        assert_eq!(
            trust_label("Edge", &test(DevelopmentHostKeyStatus::Unknown)).unwrap(),
            "Trust SHA256:observed for Edge"
        );
        let changed = test(DevelopmentHostKeyStatus::Changed {
            trusted: "SHA256:old".into(),
        });
        assert_eq!(
            trust_label("Edge", &changed).unwrap(),
            "Replace trusted key for Edge with SHA256:observed"
        );
        let summary = test_summary(&changed);
        assert!(summary.contains("CHANGED") && summary.contains("SHA256:old"));
        assert!(summary.contains("credentials were not sent"));
    }

    #[test]
    fn only_the_active_method_secret_fields_are_visible() {
        use DevelopmentBastionAuth::*;
        assert!(field_visible(Password, "bastion-password"));
        assert!(!field_visible(Password, "bastion-passphrase"));
        assert!(!field_visible(Password, "bastion-key-content"));
        assert!(field_visible(PrivateKeyPath, "bastion-key-path"));
        assert!(field_visible(PrivateKeyPath, "bastion-passphrase"));
        assert!(!field_visible(PrivateKeyPath, "bastion-password"));
        assert!(field_visible(PrivateKeyContent, "bastion-key-content"));
        assert!(!field_visible(PrivateKeyContent, "bastion-key-path"));
        assert!(field_visible(PrivateKeyContent, "bastion-name"));
    }
}

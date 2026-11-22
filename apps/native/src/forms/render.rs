//! Full-window form pages: a centred column with a header (title, subtitle
//! and, on dismissable pages, a close button aligned to the column), titled
//! sections of labelled inputs with inline errors, a tone-aware message and a
//! footer of actions. Short pages centre vertically; the long editors
//! (connection, bastions) keep their footer in a sticky band at the bottom.
use super::engine::Engine;
use super::validation::{self, Errors};
use super::*;
use crate::{style, ui};
use gpui::{AnyElement, Div, ElementId, MouseButton, Stateful, svg};

/// Height of one segment inside a page `chip_row` (the group is `FIELD`).
const SEGMENT: f32 = style::FIELD - 4.;
/// Gap between a checkbox and its label.
const CHECK_GAP: f32 = style::S4;
/// Checkbox glyph width (`ui::check_box`).
const CHECK: f32 = 12.;
/// Height of the sticky action band of the long editor pages.
const ACTION_BAND: f32 = 52.;

/// One segment of a page's exclusive choice (engine, environment, TLS
/// mode…) inside a `chip_row`: the selected segment is `raised` with `text`
/// and a line border, the rest `dim`. `dot` adds a colour marker, used for
/// environments.
pub(super) fn chip(
    id: impl Into<ElementId>,
    label: SharedString,
    selected: bool,
    enabled: bool,
    dot: Option<u32>,
) -> Stateful<Div> {
    let element = div()
        .id(id)
        .aria_label(label.clone())
        .flex_none()
        .h(px(SEGMENT))
        .px(px(style::S5 - 2.))
        .flex()
        .items_center()
        .gap(px(style::S3))
        .rounded(px(style::R_SM))
        .border_1()
        .border_color(if selected {
            style::line()
        } else {
            gpui::transparent_black().into()
        })
        .when(selected, |chip| chip.bg(style::raised()))
        .text_sm()
        .whitespace_nowrap()
        .text_color(if selected {
            style::text()
        } else if enabled {
            style::dim()
        } else {
            style::faint()
        })
        .focus(|s| s.shadow(vec![ui::ring()]))
        .when_some(dot, |chip, color| {
            chip.child(
                div()
                    .size(px(6.))
                    .rounded_full()
                    .bg(style::with_alpha(color, 0xff)),
            )
        })
        .child(label);
    if enabled {
        ui::press(
            element
                .cursor_pointer()
                .hover(|s| s.text_color(style::text())),
        )
    } else {
        element.opacity(0.5)
    }
}

/// A checkbox row. Labels such as `TLS: on` keep only the subject; the box
/// shows the state and AX reports it as toggled.
pub(super) fn toggle_row(
    id: impl Into<ElementId>,
    label: SharedString,
    checked: bool,
    enabled: bool,
) -> Stateful<Div> {
    let subject: SharedString = label
        .split_once(": ")
        .map_or(label.clone(), |(subject, _)| subject.to_owned().into());
    div()
        .id(id)
        .aria_label(subject.clone())
        .flex_none()
        .min_h(px(style::ROW - 4.))
        .flex()
        .items_start()
        .gap(px(CHECK_GAP))
        .rounded(px(style::R_SM))
        .text_sm()
        .text_color(if enabled {
            style::text()
        } else {
            style::faint()
        })
        .when(enabled, |row| row.cursor_pointer())
        .focus(|s| s.shadow(vec![ui::ring()]))
        .child(ui::check_box(checked))
        .child(div().flex_1().min_w_0().child(subject))
}

/// A labelled exclusive choice: the label above its `chip_row`.
pub(super) fn choices(label: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(style::S3))
        .child(div().text_sm().text_color(style::dim()).child(label))
}

/// The bordered group that holds a page's `chip` segments (a segment group
/// at `FIELD` height). Sizes to its segments instead of the column.
pub(super) fn chip_row() -> Div {
    div()
        .flex_none()
        .self_start()
        .p(px(1.))
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(1.))
        .rounded(px(style::R_MD))
        .border_1()
        .border_color(style::line())
        .bg(style::bg())
}

/// A quiet note under a control: `dim`, one idea.
pub(super) fn hint(text: impl Into<SharedString>) -> Div {
    div().text_sm().text_color(style::dim()).child(text.into())
}

pub(super) fn grid(columns: u16) -> Div {
    div()
        .grid()
        .grid_cols(columns)
        .gap_x(px(style::S5))
        .gap_y(px(style::S5))
}

/// TLS client files, hidden while TLS is disabled (values are kept).
pub(super) const TLS_FILES: [&str; 3] = ["root-cert", "client-cert", "client-key"];

impl Form {
    /// The labelled input for `key`, or `None` when the field is hidden.
    pub(super) fn text_field(&self, key: &str, errors: &Errors, tall: bool) -> Option<Div> {
        self.text_field_with(key, errors, tall, None)
    }

    /// Whether `key` is a field of this form and currently shown.
    fn shows_field(&self, key: &str) -> bool {
        self.fields.iter().any(|field| field.key == key) && self.field_visible(key)
    }

    /// `text_field` with an optional trailing control inside the well (a
    /// file chooser icon button).
    fn text_field_with(
        &self,
        key: &str,
        errors: &Errors,
        tall: bool,
        action: Option<AnyElement>,
    ) -> Option<Div> {
        let field = self.fields.iter().find(|field| field.key == key)?;
        if !self.field_visible(key) {
            return None;
        }
        let error = validation::error_for(errors, key);
        let note = error
            .map(|text| (SharedString::from(text.to_owned()), true))
            .or_else(|| {
                (key == "password" && matches!(self.kind, Kind::Connection { id: Some(_) }))
                    .then(|| ("Leave blank to keep the saved password".into(), false))
            });
        let focused = self.focused_field == Some(field.key);
        let has_action = action.is_some();
        Some(ui::labelled(
            field.label,
            ui::input_well(error.is_some(), focused)
                .when(tall, |frame| {
                    frame.h(px(96.)).items_start().py(px(style::S3))
                })
                .when(has_action, |frame| frame.pr(px(style::S1)))
                .child(div().flex_1().min_w_0().child(field.accessible.clone()))
                .children(action),
            note,
        ))
    }

    fn heading(&self) -> (SharedString, Option<SharedString>) {
        match &self.kind {
            Kind::Credentials(settings) => match settings.state {
                DevelopmentCredentialState::NeedsUnlock => (
                    "Unlock credentials".into(),
                    Some("Enter your credential password to load saved connections.".into()),
                ),
                DevelopmentCredentialState::NeedsOnboarding => (
                    "Set up dbunk".into(),
                    Some("Choose how dbunk stores saved database passwords.".into()),
                ),
                DevelopmentCredentialState::NeedsRecovery => (
                    "Recover credentials".into(),
                    Some("An interrupted credential change needs recovery. Stored data is preserved.".into()),
                ),
                DevelopmentCredentialState::Ready => (
                    "Credential storage".into(),
                    Some("Changing storage moves every saved password and closes open sessions.".into()),
                ),
            },
            Kind::Connection { id } => (
                if id.is_some() {
                    format!("Edit {} connection", self.engine.label()).into()
                } else {
                    "New connection".into()
                },
                Some(
                    "Saved locally. Passwords follow your credential storage setting.".into(),
                ),
            ),
            Kind::Rename => ("Rename query".into(), None),
            Kind::OpenTable => ("Open table".into(), None),
            Kind::Delete(_) => ("Delete connection".into(), None),
            Kind::Discard => ("Unsaved drafts".into(), None),
            Kind::ResetWorkspace => ("Reset saved drafts".into(), None),
            Kind::Bastions(_) => (
                "Bastion servers".into(),
                Some("SSH hops that connections can tunnel through.".into()),
            ),
        }
    }

    fn page_width(&self) -> f32 {
        match self.kind {
            Kind::Credentials(ref settings)
                if settings.state == DevelopmentCredentialState::NeedsUnlock =>
            {
                380.
            }
            Kind::Credentials(_) => 560.,
            Kind::Connection { .. } | Kind::Bastions(_) => 640.,
            _ => 420.,
        }
    }

    fn credentials_view(&mut self, errors: &Errors, cx: &Context<Self>) -> Div {
        let Kind::Credentials(settings) = &self.kind else {
            return div();
        };
        let state = settings.state;
        let current = settings.mode;
        let keychain = settings.keychain_unavailable.clone();
        let mut content = div().flex().flex_col().gap(px(style::S6));
        if state == DevelopmentCredentialState::NeedsRecovery {
            return content.child(self.button(
                "Recover credentials",
                FormAction::Recover,
                false,
                cx,
            ));
        }
        if self.confirm_reset {
            return content
                .child(ui::error_banner(
                    "reset-warning",
                    "Reset deletes every saved database password. Open sessions close and active transactions roll back. Connections and SQL drafts are kept.",
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(style::S4))
                        .child(self.button(
                            "Reset storage",
                            FormAction::ConfirmReset,
                            false,
                            cx,
                        ))
                        .child(self.button("Back", FormAction::Cancel, false, cx)),
                );
        }
        if state == DevelopmentCredentialState::NeedsUnlock {
            return content
                .when_some(self.text_field("password", errors, false), |c, f| {
                    c.child(f)
                })
                .child(div().flex().child(self.button(
                    "Forgot password?",
                    FormAction::Reset,
                    false,
                    cx,
                )));
        }
        // The Keychain could not be read: say why and offer a retry. Storage
        // stays Ready, so the rest of the page still changes or resets it.
        if let Some(reason) = keychain {
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(style::S4))
                    .child(ui::error_banner(
                        "keychain-unavailable",
                        super::keychain_notice(&reason),
                    ))
                    .child(div().flex().child(self.button(
                        "Retry keychain access",
                        FormAction::RetryKeychain,
                        false,
                        cx,
                    ))),
            );
        }
        let cards = [
            (
                DevelopmentStorageMode::EncryptedSqlite,
                "Encrypted SQLite",
                "Passwords are encrypted in dbunk.sqlite with an app password you enter once per session.",
                "icons/file_lock.svg",
                Some("Recommended"),
            ),
            (
                DevelopmentStorageMode::Keychain,
                "OS keychain",
                "Uses the macOS keychain. Secure, but macOS may ask for permission after rebuilds.",
                "icons/user_check.svg",
                None,
            ),
            (
                DevelopmentStorageMode::PlainSqlite,
                "Unencrypted SQLite",
                "Passwords are stored as plain text in dbunk.sqlite. Anyone with file access can read them.",
                "icons/file.svg",
                None,
            ),
        ];
        let mut list = div()
            .id("storage-modes")
            .role(Role::RadioGroup)
            .aria_label("Credential storage")
            .flex()
            .flex_col()
            .gap(px(style::S4));
        for (mode, title, body, icon, badge) in cards {
            let selected = self.mode == mode;
            let title = if Some(mode) == current && state == DevelopmentCredentialState::Ready {
                format!("{title} · current")
            } else {
                title.to_owned()
            };
            list = list.child(self.mode_card(mode, title, body, icon, badge, selected, cx));
        }
        content = content
            .child(list)
            .child(hint("Credential encryption does not encrypt SQL drafts."));
        if self.mode == DevelopmentStorageMode::EncryptedSqlite {
            content = content.child(
                grid(2)
                    .when_some(self.text_field("password", errors, false), |g, f| {
                        g.child(f)
                    })
                    .when_some(self.text_field("confirm", errors, false), |g, f| g.child(f)),
            );
        }
        if validation::needs_acknowledgement(self.mode) {
            let text = if self.mode == DevelopmentStorageMode::EncryptedSqlite {
                "I understand this password cannot be recovered. If I forget it, credential storage must be reset and saved database passwords are lost."
            } else {
                "I understand database passwords will be stored as plain text in the local SQLite database."
            };
            let error = validation::error_for(errors, "acknowledge").map(str::to_owned);
            content = content.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(style::S2))
                    .p(px(style::S5))
                    .rounded(px(style::R_MD))
                    .border_1()
                    .border_color(if error.is_some() {
                        style::bad_line()
                    } else {
                        style::line()
                    })
                    .bg(style::panel())
                    .child(self.button(text, FormAction::Acknowledge, self.acknowledged, cx))
                    .when_some(error, |c, error| {
                        c.child(
                            div()
                                .pl(px(CHECK + CHECK_GAP))
                                .text_size(px(style::FONT_SMALL))
                                .text_color(style::bad_text())
                                .child(error),
                        )
                    }),
            );
        }
        if state == DevelopmentCredentialState::Ready {
            content = content.child(div().flex().child(self.button(
                "Reset saved passwords…",
                FormAction::Reset,
                false,
                cx,
            )));
        }
        content
    }

    #[allow(clippy::too_many_arguments)]
    fn mode_card(
        &mut self,
        mode: DevelopmentStorageMode,
        title: String,
        body: &'static str,
        icon: &'static str,
        badge: Option<&'static str>,
        selected: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let action = FormAction::Mode(mode);
        let focus = self
            .controls
            .entry(format!("mode-{mode:?}"))
            .or_insert_with(|| cx.focus_handle())
            .clone();
        self.visible_controls.push(focus.clone());
        let enabled = !self.busy;
        let weak = cx.weak_entity();
        ui::press(ui::choice_card(
            SharedString::from(format!("mode-card-{mode:?}")),
            title,
            body,
            icon,
            badge,
            selected,
        ))
        .track_focus(&focus)
        .tab_index(0)
        .tab_stop(enabled)
        .a11y_synthetic_children(move |builder| {
            builder.parent_node().set_toggled(Toggled::from(selected));
        })
        .when(enabled, |card| {
            card.on_click(cx.listener(move |this, _, window, cx| this.activate(action, window, cx)))
        })
        .on_a11y_action(Action::Click, move |_, window, cx| {
            let _ = weak.update(cx, |this, cx| this.activate(action, window, cx));
        })
        .into_any_element()
    }

    fn connection_view(&mut self, errors: &Errors, cx: &mut Context<Self>) -> Div {
        let engine = self.engine;
        let field = |this: &Self, key| this.text_field(key, errors, false);
        let mut content = div().flex().flex_col().gap(px(style::S7));
        if self.engine_selectable() {
            let mut picker = chip_row()
                .id("engine-picker")
                .role(Role::RadioGroup)
                .aria_label("Engine");
            for value in Engine::ALL {
                picker = picker.child(self.button(
                    value.label(),
                    FormAction::Engine(value),
                    self.engine == value,
                    cx,
                ));
            }
            content = content.child(choices("Engine").child(picker));
        }

        // Connection
        let mut endpoint = grid(4);
        if let Some(name) = field(self, "name") {
            endpoint = endpoint.child(name.col_span_full());
        }
        if self.shows_field("path") {
            let pick = self.button("Choose file…", FormAction::Pick("path"), false, cx);
            if let Some(path) = self.text_field_with("path", errors, false, Some(pick)) {
                endpoint = endpoint.child(path.col_span_full());
            }
        }
        if let Some(host) = field(self, "host") {
            endpoint = endpoint.child(host.col_span(3));
        }
        if let Some(port) = field(self, "port") {
            endpoint = endpoint.child(port);
        }
        for key in ["database", "url-path", "db-number"] {
            if let Some(input) = field(self, key) {
                endpoint = endpoint.child(input.col_span(2));
            }
        }
        for key in ["user", "password"] {
            if let Some(input) = field(self, key) {
                endpoint = endpoint.child(input.col_span(2));
            }
        }
        let mut section = ui::section("Connection").child(endpoint);
        if engine == Engine::Sqlite {
            section = section.child(hint("The file must already exist; dbunk never creates it."));
        }
        content = content.child(section);

        // Safety
        let mut environments = chip_row();
        for (label, value) in [
            ("Development", DevelopmentEnvironment::Development),
            ("Test", DevelopmentEnvironment::Test),
            ("Staging", DevelopmentEnvironment::Staging),
            ("Production", DevelopmentEnvironment::Production),
        ] {
            environments = environments.child(self.button(
                label,
                FormAction::Environment(value),
                self.environment == value,
                cx,
            ));
        }
        let mut safe = chip_row();
        for (label, value) in [
            ("Inherit", DevelopmentSafeMode::Inherit),
            ("Disabled", DevelopmentSafeMode::Disabled),
            ("Protected", DevelopmentSafeMode::Protected),
            ("Strict", DevelopmentSafeMode::Strict),
        ] {
            safe = safe.child(self.button(label, FormAction::Safe(value), self.safe == value, cx));
        }
        let read_only = self.read_only;
        let favorite = self.favorite;
        content = content.child(
            ui::section("Safety")
                .child(choices("Environment").child(environments))
                .child(choices("Safe mode").child(safe))
                .child(
                    div()
                        .flex()
                        .gap(px(style::S7))
                        .child(self.button("Read-only", FormAction::ReadOnly, read_only, cx))
                        .child(self.button("Favorite", FormAction::Favorite, favorite, cx)),
                ),
        );

        // Organization
        content = content.child(
            ui::section("Organization").child(
                grid(3)
                    .when_some(field(self, "project"), |g, f| g.child(f))
                    .when_some(field(self, "folder"), |g, f| g.child(f))
                    .when_some(field(self, "color"), |g, f| g.child(f)),
            ),
        );

        // Transport
        if let Some(view) = self.engine_view(cx) {
            content = content.child(ui::section("Transport").child(view));
        }
        if engine == Engine::Postgres {
            let mut tls = chip_row();
            for (label, value) in [
                ("Disable", DevelopmentTlsMode::Disable),
                ("Prefer", DevelopmentTlsMode::Prefer),
                ("Require", DevelopmentTlsMode::Require),
                ("Verify CA", DevelopmentTlsMode::VerifyCa),
                ("Verify full", DevelopmentTlsMode::VerifyFull),
            ] {
                tls = tls.child(self.button(label, FormAction::Tls(value), self.tls == value, cx));
            }
            let mut files = div().flex().flex_col().gap(px(style::S5));
            for (key, pick) in TLS_FILES.into_iter().zip([
                "Choose root certificate",
                "Choose client certificate",
                "Choose client key",
            ]) {
                if self.shows_field(key) {
                    let action = self.button(pick, FormAction::Pick(key), false, cx);
                    if let Some(input) = self.text_field_with(key, errors, false, Some(action)) {
                        files = files.child(input);
                    }
                }
            }
            content = content.child(
                ui::section("TLS")
                    .child(choices("Mode").child(tls))
                    .child(files)
                    .when_some(field(self, "server-name"), |s, f| s.child(f)),
            );
        }
        if engine.tunnels()
            && let Some(view) = self.tunnel_view(cx)
        {
            content = content.child(ui::section("SSH tunnel").child(view));
        }
        if engine == Engine::Postgres {
            let mut driver = grid(2);
            for key in [
                "statement-timeout",
                "idle-timeout",
                "connect-timeout",
                "keepalive",
                "search-path",
                "role",
            ] {
                if let Some(input) = field(self, key) {
                    driver = driver.child(input);
                }
            }
            content = content.child(ui::section("Driver options").child(driver));
        }
        if let Some(view) = self.diagnosis.as_ref().and_then(diagnosis::State::view) {
            content = content.child(ui::section("Connection test").child(view));
        }
        content
    }

    fn simple_view(&mut self, errors: &Errors) -> Div {
        let mut content = div().flex().flex_col().gap(px(style::S5));
        if let Some(prompt) = &self.prompt {
            content = content.child(div().text_color(style::text()).child(prompt.clone()));
        }
        let keys: Vec<&'static str> = self.fields.iter().map(|field| field.key).collect();
        for key in keys {
            if let Some(input) = self.text_field(key, errors, false) {
                content = content.child(input);
            }
        }
        content
    }

    fn message_view(&self) -> Option<AnyElement> {
        let message = self.message.clone()?;
        if self.tone == Tone::Error {
            return Some(
                ui::shake(
                    ("form-error", self.message_seq),
                    ui::error_banner("form-message", message),
                )
                .into_any_element(),
            );
        }
        let (icon, color, fill) = if self.tone == Tone::Success {
            ("icons/check.svg", style::ok(), style::ok_fill())
        } else {
            ("icons/info.svg", style::dim(), style::panel())
        };
        Some(
            div()
                .id("form-message")
                .role(Role::Status)
                .aria_label(message.clone())
                .a11y_synthetic_children(|builder| {
                    builder.parent_node().set_live(Live::Polite);
                })
                .flex()
                .items_start()
                .gap(px(style::S3))
                .px(px(style::S4))
                .py(px(style::S3))
                .rounded(px(style::R_MD))
                .border_1()
                .border_color(style::line())
                .bg(fill)
                .text_sm()
                .text_color(color)
                .child(
                    svg()
                        .path(icon)
                        .mt(px(2.))
                        .size(px(style::ICON))
                        .flex_none()
                        .text_color(color),
                )
                .child(div().flex_1().min_w_0().child(message))
                .into_any_element(),
        )
    }

    fn footer(&mut self, cx: &Context<Self>) -> Option<Div> {
        let primary = match &self.kind {
            Kind::OpenTable => Some(("Open", FormAction::Submit)),
            Kind::Rename => Some(("Rename", FormAction::Submit)),
            Kind::Delete(_) => Some(("Delete connection", FormAction::Delete)),
            Kind::Discard => Some(("Discard drafts and close", FormAction::Discard)),
            Kind::ResetWorkspace => Some(("Reset saved drafts", FormAction::Discard)),
            Kind::Credentials(_) if self.confirm_reset => None,
            Kind::Credentials(settings) => match settings.state {
                DevelopmentCredentialState::NeedsRecovery => None,
                DevelopmentCredentialState::NeedsUnlock => Some(("Unlock", FormAction::Submit)),
                DevelopmentCredentialState::NeedsOnboarding => {
                    Some(("Continue", FormAction::Submit))
                }
                DevelopmentCredentialState::Ready => Some(("Save", FormAction::Submit)),
            },
            Kind::Bastions(state) if !state.editing() => None,
            Kind::Bastions(_) => Some(("Save bastion", FormAction::Submit)),
            Kind::Connection { id: None } => Some(("Save connection", FormAction::Submit)),
            Kind::Connection { .. } => Some(("Save", FormAction::Submit)),
        };
        let mut leading = div().flex().items_center().gap(px(style::S4));
        if matches!(self.kind, Kind::Connection { id: None }) && self.engine == Engine::Postgres {
            leading = leading.child(self.button(
                "Import URI from clipboard",
                FormAction::ImportUri,
                false,
                cx,
            ));
        }
        if matches!(self.kind, Kind::Connection { .. }) {
            leading = leading.child(self.button("Test connection", FormAction::Test, false, cx));
        }
        if matches!(&self.kind, Kind::Bastions(state) if state.editing()) {
            leading = leading.child(self.button(
                "Back to Bastion Servers",
                FormAction::Bastion(bastions::Action::Back),
                false,
                cx,
            ));
        }
        let mut trailing = div().flex().items_center().gap(px(style::S4));
        if self.busy {
            trailing = trailing.child(hint("Working…"));
        }
        if self.dismissible() && !self.confirm_reset {
            let cancel = if matches!(self.kind, Kind::Bastions(_)) {
                "Close"
            } else {
                "Cancel"
            };
            trailing = trailing.child(self.button(cancel, FormAction::Cancel, false, cx));
        }
        if let Some((label, action)) = primary {
            trailing = trailing.child(self.button(label, action, false, cx));
        }
        Some(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(style::S4))
                .child(leading)
                .child(trailing),
        )
    }

    /// The close button at the right of the page header, aligned to the
    /// column, when the page can be dismissed.
    fn close_button(&mut self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.dismissible() {
            return None;
        }
        let focus = self
            .controls
            .entry("Close page".into())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let weak = cx.weak_entity();
        Some(
            ui::press(
                div()
                    .id("form-close")
                    .role(Role::Button)
                    .aria_label("Close")
                    .track_focus(&focus)
                    .tab_index(0)
                    .flex_none()
                    .size(px(style::CONTROL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(style::R_SM))
                    .text_color(style::dim())
                    .cursor_pointer()
                    .hover(|s| s.bg(style::hover()).text_color(style::text()))
                    .focus(|s| s.text_color(style::text()).shadow(vec![ui::ring()])),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .tooltip(ui::tooltip("Close  esc"))
            .tooltip_show_delay(ui::tooltip_delay())
            .on_click(
                cx.listener(|this, _, window, cx| this.activate(FormAction::Cancel, window, cx)),
            )
            .on_a11y_action(Action::Click, move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.activate(FormAction::Cancel, window, cx));
            })
            .child(
                svg()
                    .path("icons/close.svg")
                    .size(px(style::ICON))
                    .text_color(style::dim()),
            )
            .into_any_element(),
        )
    }

    fn page_key(&self) -> &'static str {
        match &self.kind {
            Kind::Credentials(settings) => match settings.state {
                DevelopmentCredentialState::NeedsUnlock => "unlock",
                DevelopmentCredentialState::NeedsOnboarding => "onboarding",
                DevelopmentCredentialState::NeedsRecovery => "recovery",
                DevelopmentCredentialState::Ready => "credentials",
            },
            Kind::Connection { .. } => "connection",
            Kind::Rename => "rename",
            Kind::OpenTable => "open-table",
            Kind::Delete(_) => "delete",
            Kind::Discard => "discard",
            Kind::ResetWorkspace => "reset-workspace",
            Kind::Bastions(_) => "bastions",
        }
    }
}

impl Form {
    /// The long editors keep their actions in a sticky band at the bottom.
    fn sticky_footer(&self) -> bool {
        matches!(self.kind, Kind::Connection { .. } | Kind::Bastions(_))
    }
}

impl Render for Form {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_controls.clear();
        self.focused_field = self
            .fields
            .iter()
            .find(|field| field.editor.focus_handle(cx).is_focused(window))
            .map(|field| field.key);
        let errors = if self.attempted {
            self.field_errors(cx)
        } else {
            Vec::new()
        };
        let (title, subtitle) = self.heading();
        let width = self.page_width();
        let gate = matches!(self.kind, Kind::Credentials(_)) && !self.dismissible();
        let sticky = self.sticky_footer();
        let body = match &self.kind {
            Kind::Credentials(_) => self.credentials_view(&errors, cx),
            Kind::Connection { .. } => self.connection_view(&errors, cx),
            Kind::Bastions(_) => self.bastion_view(cx),
            _ => self.simple_view(&errors),
        };
        let message = self.message_view();
        let footer = self.footer(cx);
        let close = self.close_button(cx);
        let header = div()
            .flex()
            .items_start()
            .gap(px(style::S5))
            .when(gate, |header| {
                header.child(
                    div()
                        .flex_none()
                        .size(px(32.))
                        .rounded(px(style::R_MD))
                        .border_1()
                        .border_color(style::primary_line())
                        .bg(style::primary_fill())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .path("icons/lock.svg")
                                .size(px(14.))
                                .text_color(style::accent()),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(style::S2))
                    .child(
                        div()
                            .text_size(px(style::FONT_TITLE))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(style::text())
                            .child(title.clone()),
                    )
                    .when_some(subtitle, |h, subtitle| {
                        h.child(div().text_sm().text_color(style::dim()).child(subtitle))
                    }),
            )
            .children(close);
        let action_row = |footer: Div| {
            div()
                .flex()
                .flex_col()
                .gap(px(style::S4))
                .children(message)
                .child(footer)
        };
        let (inline_actions, band) = match footer {
            Some(footer) if sticky => (None, Some(action_row(footer))),
            Some(footer) => (
                Some(
                    div()
                        .pt(px(style::S5))
                        .border_t_1()
                        .border_color(style::line_soft())
                        .child(action_row(footer)),
                ),
                None,
            ),
            None => (Some(action_row(div())), None),
        };
        let page = div()
            .w(px(width))
            .max_w_full()
            .flex()
            .flex_col()
            .gap(px(style::S7))
            .child(header)
            .child(body)
            .children(inline_actions);
        div()
            .id("native-form")
            .role(Role::Dialog)
            .aria_label(title)
            .size_full()
            .flex()
            .flex_col()
            .bg(style::bg())
            .text_color(style::text())
            .text_size(px(style::FONT))
            .capture_key_down(cx.listener(Self::key))
            .child(
                // Titlebar strip: keeps the window movable clear of the
                // traffic lights.
                div()
                    .id("form-titlebar")
                    .flex_none()
                    .h(px(style::BAR))
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.titlebar_double_click();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
            .child(
                div()
                    .id("form-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            // Short pages sit a little above the optical
                            // centre; long editors start at the top.
                            .when(!sticky, |column| {
                                column.min_h(gpui::relative(1.)).justify_center()
                            })
                            .flex()
                            .flex_col()
                            .items_center()
                            .px(px(style::S7))
                            .pt(px(if sticky { style::S4 } else { style::S7 }))
                            .pb(px(if sticky { style::S7 } else { 64. }))
                            .child(ui::appear(
                                SharedString::from(format!("form-page-{}", self.page_key())),
                                page,
                            )),
                    ),
            )
            .when_some(band, |form, band| {
                form.child(
                    div()
                        .flex_none()
                        .min_h(px(ACTION_BAND))
                        .px(px(style::S7))
                        .py(px(style::S5))
                        .flex()
                        .flex_col()
                        .items_center()
                        .border_t_1()
                        .border_color(style::line_soft())
                        .bg(style::bg())
                        .child(div().w(px(width)).max_w_full().child(band)),
                )
            })
    }
}

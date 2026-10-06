//! C09.a-c Managed Servers surface for general profiles. Provisioning and
//! destruction show an explicit review of exactly what is created or deleted;
//! the backend recomputes that review and refuses any difference.
use crate::{accessible_editor::AccessibleEditor, controller::Host};
use dbunk_lib::backend::managed_servers::{
    DockerAvailability, MANAGED_POSTGRES_VERSIONS, ManagedContainerStatus, ManagedDestroy,
    ManagedDestroyPlan, ManagedProvisionPlan, ManagedProvisionRequest, ManagedServerList,
    ManagedServerSummary,
};
use editor::Editor;
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Role, SharedString, Task,
    Window,
    accesskit::{Action, Live, Toggled},
    div,
    prelude::*,
    px,
};
use std::{collections::HashMap, sync::Arc};

pub enum ManagedEvent {
    /// `settle` names connections whose server was stopped or destroyed.
    Closed { changed: bool, settle: Vec<String> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RowOp {
    Start,
    Stop,
    Destroy,
    Recreate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    Check,
    Refresh,
    Compose,
    Version(&'static str),
    Review,
    Provision,
    Back,
    Row(RowOp, usize),
    ConfirmDestroy,
    Close,
}

enum Outcome {
    Listed(ManagedServerList),
    Docker(DockerAvailability),
    Planned(ManagedProvisionPlan),
    Changed(ManagedServerList, String),
    DestroyReview(ManagedDestroyPlan),
}

type Job = futures_util::future::BoxFuture<'static, Result<Outcome, String>>;

struct Field {
    label: &'static str,
    editor: Entity<Editor>,
    accessible: Entity<AccessibleEditor>,
}

pub struct ManagedServersView {
    host: Arc<Host>,
    list: Option<ManagedServerList>,
    docker: Option<DockerAvailability>,
    composing: bool,
    version: &'static str,
    fields: Vec<Field>,
    plan: Option<ManagedProvisionPlan>,
    destroy: Option<ManagedDestroyPlan>,
    busy: bool,
    message: Option<String>,
    changed: bool,
    settle: Vec<String>,
    controls: HashMap<String, FocusHandle>,
    visible_controls: Vec<FocusHandle>,
    task: Option<Task<()>>,
}

impl EventEmitter<ManagedEvent> for ManagedServersView {}

/// Which row actions apply. Conflicts and unknown status offer none: the
/// user resolves Docker first, then refreshes.
pub(crate) fn row_actions(status: &ManagedContainerStatus) -> &'static [RowOp] {
    match status {
        ManagedContainerStatus::Running => &[RowOp::Stop, RowOp::Destroy],
        ManagedContainerStatus::Stopped { .. } => &[RowOp::Start, RowOp::Destroy],
        ManagedContainerStatus::Missing => &[RowOp::Recreate, RowOp::Destroy],
        ManagedContainerStatus::Conflict | ManagedContainerStatus::Unknown => &[],
    }
}

pub(crate) fn status_label(status: &ManagedContainerStatus) -> String {
    match status {
        ManagedContainerStatus::Running => "running".into(),
        ManagedContainerStatus::Stopped { state } => format!("stopped ({state})"),
        ManagedContainerStatus::Missing => "container missing".into(),
        ManagedContainerStatus::Conflict => {
            "several containers carry its labels; resolve in Docker".into()
        }
        ManagedContainerStatus::Unknown => "status unknown".into(),
    }
}

pub(crate) fn short_id(id: &str) -> &str {
    &id[..id.len().min(12)]
}

pub(crate) fn row_summary(server: &ManagedServerSummary) -> String {
    let volume = match server.volume_present {
        Some(true) => "data volume present",
        Some(false) => "data volume missing",
        None => "data volume unknown",
    };
    let connection = server
        .connection
        .as_ref()
        .map_or("no connection".to_owned(), |c| {
            format!("connection {}", c.name)
        });
    format!(
        "{}: {} on 127.0.0.1:{} · {} · {} · {}",
        server.name,
        server.image,
        server.port,
        status_label(&server.status),
        volume,
        connection
    )
}

/// Blank selects an automatic port.
pub(crate) fn parse_port(value: &str) -> Result<Option<u16>, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(port) if port >= 1024 => Ok(Some(port)),
        _ => Err("Port must be blank or between 1024 and 65535".into()),
    }
}

pub(crate) fn provision_review(plan: &ManagedProvisionPlan) -> String {
    format!(
        "Provision {name}? This pulls {image} if needed and creates a NEW Docker volume {volume} and container {container}, published only on {binding}. A generated password is saved with your credential storage, and a connection named {connection} (database {database}, user {user}) is added. Nothing existing is adopted; any failure removes what was created.",
        name = plan.name,
        image = plan.image,
        volume = plan.volume_name,
        container = plan.container_name,
        binding = plan.host_binding,
        connection = plan.connection_name,
        database = plan.database,
        user = plan.user,
    )
}

pub(crate) fn destroy_review(plan: &ManagedDestroyPlan) -> String {
    let mut deleted = Vec::new();
    if let Some(container) = &plan.container_id {
        deleted.push(format!("container {}", short_id(container)));
    }
    if let Some(volume) = &plan.volume {
        deleted.push(format!("data volume {volume} (all databases in it)"));
    }
    if let Some(connection) = &plan.connection {
        deleted.push(format!(
            "connection {} and its saved password",
            connection.name
        ));
    }
    deleted.push("the managed server record".into());
    format!(
        "Destroy {}? This permanently deletes: {}. Open sessions on its connection close now. This cannot be undone.",
        plan.name,
        deleted.join(", ")
    )
}

impl ManagedServersView {
    pub fn new(host: Arc<Host>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            host,
            list: None,
            docker: None,
            composing: false,
            version: MANAGED_POSTGRES_VERSIONS[1],
            fields: Vec::new(),
            plan: None,
            destroy: None,
            busy: false,
            message: None,
            changed: false,
            settle: Vec::new(),
            controls: HashMap::from([("Close".into(), cx.focus_handle())]),
            visible_controls: Vec::new(),
            task: None,
        };
        let backend = view.host.backend.clone();
        view.run(
            Box::pin(async move { backend.managed_servers().await.map(Outcome::Listed) }),
            window,
            cx,
        );
        view
    }

    pub fn focus(&self, window: &mut Window, cx: &mut gpui::App) {
        if let Some(close) = self.controls.get("Close") {
            window.focus(close, cx);
        }
    }

    fn run(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        let work = self.host.runtime.spawn(job);
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|_| Err("Operation could not finish".into()));
            let _ = this.update_in(cx, |view, window, cx| view.finish(result, window, cx));
        }));
        cx.notify();
    }

    fn finish(
        &mut self,
        result: Result<Outcome, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = false;
        self.message = match result {
            Ok(Outcome::Listed(list)) => {
                self.docker = Some(list.docker.clone());
                let message = list.observe_error.clone();
                self.list = Some(list);
                message
            }
            Ok(Outcome::Docker(status)) => {
                let message = status.describe();
                self.docker = Some(status);
                Some(message)
            }
            Ok(Outcome::Planned(plan)) => {
                self.plan = Some(plan);
                None
            }
            Ok(Outcome::Changed(list, message)) => {
                self.docker = Some(list.docker.clone());
                self.list = Some(list);
                self.changed = true;
                self.plan = None;
                self.destroy = None;
                if self.composing {
                    self.composing = false;
                    self.fields.clear();
                }
                self.focus(window, cx);
                Some(message)
            }
            Ok(Outcome::DestroyReview(plan)) => {
                self.destroy = Some(plan);
                None
            }
            Err(error) => Some(error),
        };
        cx.notify();
    }

    fn row(&self, index: usize) -> Option<ManagedServerSummary> {
        self.list.as_ref()?.servers.get(index).cloned()
    }

    fn value(&self, index: usize, cx: &gpui::App) -> String {
        self.fields
            .get(index)
            .map(|field| field.editor.read(cx).text(cx))
            .unwrap_or_default()
    }

    fn field(
        &mut self,
        label: &'static str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(value, window, cx);
            editor
        });
        let accessible = cx.new(|cx| AccessibleEditor::field(editor.clone(), label, false, cx));
        self.fields.push(Field {
            label,
            editor,
            accessible,
        });
    }

    fn activate(&mut self, op: Op, window: &mut Window, cx: &mut Context<Self>) {
        if op == Op::Close {
            cx.emit(ManagedEvent::Closed {
                changed: self.changed,
                settle: std::mem::take(&mut self.settle),
            });
            return;
        }
        if self.busy {
            return;
        }
        let backend = self.host.backend.clone();
        match op {
            Op::Close => {}
            Op::Check => {
                self.run(
                    Box::pin(
                        async move { backend.managed_docker_status().await.map(Outcome::Docker) },
                    ),
                    window,
                    cx,
                );
            }
            Op::Refresh => {
                self.run(
                    Box::pin(async move { backend.managed_servers().await.map(Outcome::Listed) }),
                    window,
                    cx,
                );
            }
            Op::Compose => {
                self.composing = true;
                self.plan = None;
                self.destroy = None;
                self.message = None;
                self.fields.clear();
                self.field("Server name", "Local PostgreSQL", window, cx);
                self.field("Host port (blank for automatic)", "", window, cx);
                if let Some(first) = self.fields.first() {
                    window.focus(&first.editor.focus_handle(cx), cx);
                }
                cx.notify();
            }
            Op::Version(version) => {
                self.version = version;
                self.plan = None;
                cx.notify();
            }
            Op::Review => {
                let port = match parse_port(&self.value(1, cx)) {
                    Ok(port) => port,
                    Err(error) => {
                        self.message = Some(error);
                        cx.notify();
                        return;
                    }
                };
                let request = ManagedProvisionRequest {
                    name: self.value(0, cx),
                    version: self.version.to_owned(),
                    port,
                };
                self.run(
                    Box::pin(async move {
                        backend
                            .plan_managed_server(request)
                            .await
                            .map(Outcome::Planned)
                    }),
                    window,
                    cx,
                );
            }
            Op::Provision => {
                let Some(plan) = self.plan.clone() else {
                    return;
                };
                self.message = Some(format!(
                    "Provisioning {}; pulling {} can take several minutes…",
                    plan.name, plan.image
                ));
                self.run(
                    Box::pin(async move {
                        let name = plan.name.clone();
                        backend.provision_managed_server(plan).await?;
                        Ok(Outcome::Changed(
                            backend.managed_servers().await?,
                            format!("{name} is running and its connection was added"),
                        ))
                    }),
                    window,
                    cx,
                );
            }
            Op::Back => {
                self.composing = false;
                self.plan = None;
                self.destroy = None;
                self.fields.clear();
                self.message = None;
                self.focus(window, cx);
                cx.notify();
            }
            Op::Row(op, index) => {
                let Some(server) = self.row(index) else {
                    return;
                };
                if !row_actions(&server.status).contains(&op) {
                    return;
                }
                let id = server.id.clone();
                let name = server.name.clone();
                if matches!(op, RowOp::Stop)
                    && let Some(connection) = &server.connection
                {
                    self.settle.push(connection.id.clone());
                }
                let job: Job = match op {
                    RowOp::Start => Box::pin(async move {
                        backend.start_managed_server(id).await?;
                        Ok(Outcome::Changed(
                            backend.managed_servers().await?,
                            format!("{name} is running"),
                        ))
                    }),
                    RowOp::Stop => Box::pin(async move {
                        backend.stop_managed_server(id).await?;
                        Ok(Outcome::Changed(
                            backend.managed_servers().await?,
                            format!("{name} stopped; its open sessions were closed"),
                        ))
                    }),
                    RowOp::Recreate => Box::pin(async move {
                        backend.recreate_managed_server(id).await?;
                        Ok(Outcome::Changed(
                            backend.managed_servers().await?,
                            format!("{name} was recreated with the same identity"),
                        ))
                    }),
                    RowOp::Destroy => Box::pin(async move {
                        backend
                            .review_managed_server_destroy(id)
                            .await
                            .map(Outcome::DestroyReview)
                    }),
                };
                self.message = None;
                self.run(job, window, cx);
            }
            Op::ConfirmDestroy => {
                let Some(plan) = self.destroy.clone() else {
                    return;
                };
                if let Some(connection) = &plan.connection {
                    self.settle.push(connection.id.clone());
                }
                let name = plan.name.clone();
                self.run(
                    Box::pin(async move {
                        match backend
                            .destroy_managed_server(plan.record_id.clone(), plan)
                            .await?
                        {
                            ManagedDestroy::Destroyed => Ok(Outcome::Changed(
                                backend.managed_servers().await?,
                                format!("{name} was destroyed"),
                            )),
                            ManagedDestroy::ReviewRequired { plan } => {
                                Ok(Outcome::DestroyReview(plan))
                            }
                        }
                    }),
                    window,
                    cx,
                );
            }
        }
    }

    fn button(
        &mut self,
        key: impl Into<String>,
        label: impl Into<SharedString>,
        op: Op,
        selected: Option<bool>,
        cx: &Context<Self>,
    ) -> gpui::AnyElement {
        let key = key.into();
        let label = label.into();
        let focus = self
            .controls
            .entry(key.clone())
            .or_insert_with(|| cx.focus_handle())
            .clone();
        self.visible_controls.push(focus.clone());
        let enabled = !self.busy || op == Op::Close;
        let weak = cx.weak_entity();
        let variant = match op {
            Op::Close => crate::ui::Variant::Ghost,
            _ => crate::ui::Variant::Secondary,
        };
        crate::ui::button(SharedString::from(key), label.clone(), variant, enabled)
            .when(selected == Some(true), |button| {
                button
                    .bg(crate::style::primary_fill())
                    .border_color(crate::style::primary_line())
                    .text_color(crate::style::primary_text())
            })
            .role(if selected.is_some() {
                Role::RadioButton
            } else {
                Role::Button
            })
            .track_focus(&focus)
            .tab_index(0)
            .tab_stop(enabled)
            .a11y_synthetic_children(move |builder| {
                if let Some(selected) = selected {
                    builder.parent_node().set_toggled(Toggled::from(selected));
                }
                if !enabled {
                    builder.parent_node().set_disabled();
                }
            })
            // GPUI fires on_click for a focused element on Enter/Space key-up.
            .on_click(cx.listener(move |this, _, window, cx| this.activate(op, window, cx)))
            .on_a11y_action(Action::Click, move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.activate(op, window, cx));
            })
            .into_any_element()
    }

    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.fields.iter().any(|field| {
            field.editor.focus_handle(cx).is_focused(window)
                && field.editor.update(cx, |editor, cx| {
                    gpui::EntityInputHandler::marked_text_range(editor, window, cx).is_some()
                })
        }) {
            return;
        }
        if event.keystroke.key == "escape" {
            if self.plan.is_some() || self.destroy.is_some() || self.composing {
                self.activate(Op::Back, window, cx);
            } else {
                self.activate(Op::Close, window, cx);
            }
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

    fn composer(&mut self, cx: &mut Context<Self>) -> gpui::Div {
        let mut content = div().flex().flex_col().gap(px(10.));
        let mut fields = div().grid().grid_cols(2).gap_x(px(12.)).gap_y(px(10.));
        for field in &self.fields {
            fields = fields.child(crate::ui::labelled(
                field.label,
                crate::ui::input_frame(false)
                    .child(div().flex_1().min_w_0().child(field.accessible.clone())),
                None,
            ));
        }
        content = content.child(fields);
        let mut versions = div().flex().flex_wrap().gap_2().child("PostgreSQL version");
        for version in MANAGED_POSTGRES_VERSIONS {
            versions = versions.child(self.button(
                format!("version-{version}"),
                format!("PostgreSQL {version}"),
                Op::Version(version),
                Some(self.version == *version),
                cx,
            ));
        }
        content = content.child(versions);
        if let Some(plan) = self.plan.clone() {
            let review = provision_review(&plan);
            content = content
                .child(
                    div()
                        .id("managed-provision-review")
                        .role(Role::Alert)
                        .aria_label(review.clone())
                        .child(review),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(self.button(
                            "provision-confirm",
                            format!("Provision {} on {}", plan.name, plan.host_binding),
                            Op::Provision,
                            None,
                            cx,
                        ))
                        .child(self.button("provision-back", "Back", Op::Back, None, cx)),
                );
        } else {
            content = content.child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button(
                        "provision-review",
                        "Review provisioning",
                        Op::Review,
                        None,
                        cx,
                    ))
                    .child(self.button("provision-back", "Back", Op::Back, None, cx)),
            );
        }
        content
    }

    fn rows(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(list) = self.list.clone() else {
            return div().child("Loading managed servers…").into_any_element();
        };
        if list.servers.is_empty() {
            return div().child("No managed servers yet.").into_any_element();
        }
        let destroy = self.destroy.clone();
        let mut rows = div()
            .id("managed-list")
            .role(Role::List)
            .aria_label("Managed servers")
            .flex()
            .flex_col()
            .gap_3();
        for (index, server) in list.servers.iter().enumerate() {
            let summary = row_summary(server);
            let mut actions = div().flex().flex_wrap().gap_2();
            for op in row_actions(&server.status) {
                let verb = match op {
                    RowOp::Start => "Start",
                    RowOp::Stop => "Stop",
                    RowOp::Destroy => "Destroy",
                    RowOp::Recreate => "Recreate",
                };
                actions = actions.child(self.button(
                    format!("managed-{op:?}-{index}"),
                    format!("{verb} {}", server.name),
                    Op::Row(*op, index),
                    None,
                    cx,
                ));
            }
            let mut row = div()
                .id(("managed-row", index))
                .role(Role::ListItem)
                .aria_label(summary.clone())
                .flex()
                .flex_col()
                .gap_1()
                .child(summary)
                .child(actions);
            if let Some(plan) = destroy.as_ref().filter(|plan| plan.record_id == server.id) {
                let review = destroy_review(plan);
                row = row
                    .child(
                        div()
                            .id(("managed-destroy-review", index))
                            .role(Role::Alert)
                            .aria_label(review.clone())
                            .child(review),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(self.button(
                                "destroy-confirm",
                                format!("Destroy {} permanently", plan.name),
                                Op::ConfirmDestroy,
                                None,
                                cx,
                            ))
                            .child(self.button(
                                "destroy-keep",
                                format!("Keep {}", plan.name),
                                Op::Back,
                                None,
                                cx,
                            )),
                    );
            }
            rows = rows.child(row);
        }
        rows.into_any_element()
    }
}

impl Render for ManagedServersView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.visible_controls.clear();
        let docker = self.docker.as_ref().map_or(
            "Docker not checked yet".to_owned(),
            DockerAvailability::describe,
        );
        let mut content = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .id("managed-docker-status")
                    .role(Role::Status)
                    .aria_label(docker.clone())
                    .child(docker),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(self.button("docker-check", "Check Docker", Op::Check, None, cx))
                    .child(self.button("managed-refresh", "Refresh", Op::Refresh, None, cx))
                    .when(!self.composing, |toolbar| {
                        toolbar.child(self.button(
                            "managed-compose",
                            "+ Provision PostgreSQL",
                            Op::Compose,
                            None,
                            cx,
                        ))
                    }),
            );
        if self.composing {
            content = content.child(self.composer(cx));
        }
        content = content.child(self.rows(cx));
        if let Some(message) = &self.message {
            content = content.child(
                div()
                    .id("managed-message")
                    .role(Role::Alert)
                    .aria_label(message.clone())
                    .a11y_synthetic_children(|builder| {
                        builder.parent_node().set_live(Live::Polite);
                    })
                    .child(message.clone()),
            );
        }
        let mut footer = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(6.))
            .pt(px(12.))
            .border_t_1()
            .border_color(crate::style::line_soft());
        if self.busy {
            footer = footer.child(
                div()
                    .text_sm()
                    .text_color(crate::style::faint())
                    .child("Working…"),
            );
        }
        footer = footer.child(self.button("Close", "Close", Op::Close, None, cx));
        div()
            .id("managed-servers")
            .role(Role::Dialog)
            .aria_label("Managed servers")
            .bg(crate::style::bg())
            .text_color(crate::style::text())
            .text_size(px(crate::style::FONT))
            .size_full()
            .overflow_y_scroll()
            .capture_key_down(cx.listener(Self::key))
            .child(div().h(px(crate::style::BAR)).flex_none())
            .child(
                div().w_full().flex().justify_center().px(px(24.)).pb(px(32.)).child(
                    div()
                        .w(px(680.))
                        .max_w_full()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(
                                    div()
                                        .text_size(px(crate::style::FONT_TITLE))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .child("Managed servers"),
                                )
                                .child(div().text_sm().text_color(crate::style::dim()).child(
                                    "Local PostgreSQL in Docker, owned by this profile. Containers are matched by their labels, never by name.",
                                )),
                        )
                        .child(content)
                        .child(footer),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbunk_lib::backend::managed_servers::ManagedConnectionRef;

    fn server(status: ManagedContainerStatus) -> ManagedServerSummary {
        ManagedServerSummary {
            id: "record".into(),
            name: "Orders".into(),
            image: "postgres:17".into(),
            version: "17".into(),
            port: 5433,
            container_name: "dbunk-native-orders-12345678".into(),
            volume_name: "dbunk-native-orders-12345678-data".into(),
            database: "orders".into(),
            user: "orders".into(),
            connection: Some(ManagedConnectionRef {
                id: "connection".into(),
                name: "Orders".into(),
            }),
            status,
            container_id: None,
            volume_present: Some(true),
            created_at: "now".into(),
        }
    }

    #[test]
    fn actions_follow_observed_status_and_offer_nothing_when_unknown() {
        use ManagedContainerStatus::*;
        assert_eq!(row_actions(&Running), &[RowOp::Stop, RowOp::Destroy]);
        assert_eq!(
            row_actions(&Stopped {
                state: "exited".into()
            }),
            &[RowOp::Start, RowOp::Destroy]
        );
        assert_eq!(row_actions(&Missing), &[RowOp::Recreate, RowOp::Destroy]);
        assert!(row_actions(&Conflict).is_empty());
        assert!(row_actions(&Unknown).is_empty());
        let summary = row_summary(&server(Running));
        assert!(summary.contains("127.0.0.1:5433") && summary.contains("running"));
    }

    #[test]
    fn port_input_is_blank_or_unprivileged() {
        assert_eq!(parse_port(" "), Ok(None));
        assert_eq!(parse_port("5440"), Ok(Some(5440)));
        assert!(parse_port("80").is_err());
        assert!(parse_port("70000").is_err());
        assert!(parse_port("abc").is_err());
    }

    #[test]
    fn reviews_name_everything_created_or_deleted() {
        let plan = ManagedProvisionPlan {
            record_id: "record".into(),
            name: "Orders".into(),
            image: "postgres:17".into(),
            version: "17".into(),
            host_binding: "127.0.0.1:5433".into(),
            port: 5433,
            container_name: "dbunk-native-orders-12345678".into(),
            volume_name: "dbunk-native-orders-12345678-data".into(),
            database: "orders".into(),
            user: "orders".into(),
            connection_name: "Orders".into(),
        };
        let review = provision_review(&plan);
        for part in [
            "postgres:17",
            "127.0.0.1:5433",
            "dbunk-native-orders-12345678-data",
            "dbunk-native-orders-12345678",
            "NEW",
        ] {
            assert!(review.contains(part), "{part}");
        }
        let destroy = ManagedDestroyPlan {
            record_id: "record".into(),
            name: "Orders".into(),
            container_id: Some("a".repeat(64)),
            volume: Some("dbunk-native-orders-12345678-data".into()),
            connection: Some(ManagedConnectionRef {
                id: "connection".into(),
                name: "Orders".into(),
            }),
        };
        let review = destroy_review(&destroy);
        assert!(review.contains(&"a".repeat(12)) && !review.contains(&"a".repeat(13)));
        assert!(review.contains("data volume dbunk-native-orders-12345678-data"));
        assert!(review.contains("connection Orders"));
        let bare = destroy_review(&ManagedDestroyPlan {
            container_id: None,
            volume: None,
            connection: None,
            ..destroy
        });
        assert!(!bare.contains("container") && !bare.contains("volume"));
        assert!(bare.contains("record"));
    }
}

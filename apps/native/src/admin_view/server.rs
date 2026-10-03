//! Server inspection shares the Administration document and serialized read lane.
use super::*;
use dbunk_lib::backend::{data::DataError, server_details::ServerDetailsSnapshot};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    Activity(ActivitySection),
    Server(ServerSection),
    Audit,
}
impl Section {
    pub(super) const ALL: [Self; 7] = [
        Self::Activity(ActivitySection::ALL[0]),
        Self::Activity(ActivitySection::ALL[1]),
        Self::Activity(ActivitySection::ALL[2]),
        Self::Server(ServerSection::ALL[0]),
        Self::Server(ServerSection::ALL[1]),
        Self::Server(ServerSection::ALL[2]),
        Self::Audit,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Activity(section) => section.label(),
            Self::Server(section) => section.label(),
            Self::Audit => "Safety overrides",
        }
    }
    pub(super) fn headings(self) -> &'static str {
        match self {
            Self::Activity(section) => section.headings(),
            Self::Server(section) => section.headings(),
            Self::Audit => "Time · Command · Statement classes",
        }
    }
}
impl AdminView {
    pub(super) fn has_capture(&self) -> bool {
        match self.section {
            Section::Activity(_) => self.snapshot.is_some(),
            Section::Server(_) => self.server.is_some(),
            Section::Audit => self.audit.capture.is_some(),
        }
    }
    pub(super) fn count(&self) -> usize {
        match self.section {
            Section::Activity(section) => self
                .snapshot
                .as_ref()
                .map_or(0, |snapshot| snapshot.count(section)),
            Section::Server(section) => self
                .server
                .as_ref()
                .map_or(0, |snapshot| snapshot.count(section)),
            Section::Audit => self
                .audit
                .capture
                .as_ref()
                .map_or(0, |capture| capture.count()),
        }
    }
    pub(super) fn selected_index(&self) -> Option<usize> {
        match self.section {
            Section::Activity(section) => {
                let index = self.selected[section.index()];
                (index < self.count()).then_some(index)
            }
            Section::Server(section) => self
                .server
                .as_ref()?
                .index_for_key(section, self.server_selected[section.index()].as_deref()?),
            Section::Audit => self
                .audit
                .capture
                .as_ref()?
                .index_for_key(self.audit.selected?),
        }
    }
    pub(super) fn select(&mut self, index: usize) {
        match self.section {
            Section::Activity(section) => self.selected[section.index()] = index,
            Section::Audit => {
                self.audit.selected = self
                    .audit
                    .capture
                    .as_ref()
                    .and_then(|capture| capture.key(index))
            }
            Section::Server(section) => {
                self.server_selected[section.index()] = self
                    .server
                    .as_ref()
                    .and_then(|snapshot| snapshot.key(section, index))
                    .map(str::to_owned)
            }
        }
    }
    pub(super) fn selected_details(&self) -> Option<String> {
        let index = self.selected_index()?;
        match self.section {
            Section::Activity(section) => self.snapshot.as_ref()?.details(section, index),
            Section::Server(section) => self.server.as_ref()?.details(section, index),
            Section::Audit => self.audit.capture.as_ref()?.details(index),
        }
    }
    pub(super) fn row_label(&self, index: usize) -> Option<String> {
        match self.section {
            Section::Activity(section) => self.snapshot.as_ref()?.row_label(section, index),
            Section::Server(section) => self.server.as_ref()?.row_label(section, index),
            Section::Audit => self.audit.capture.as_ref()?.row_label(index),
        }
    }
    pub(super) fn capture_interval(&self) -> String {
        match self.section {
            Section::Activity(_) => self.snapshot.as_ref().map(Snapshot::interval),
            Section::Server(_) => self.server.as_ref().map(ServerCapture::interval),
            Section::Audit => Some(
                "Profile-local records. Refresh starts a new reading; Older overrides continues it"
                    .into(),
            ),
        }
        .unwrap_or_else(|| "No readings collected for this section".into())
    }
    pub(super) fn capture_metrics(&self) -> String {
        match self.section {
            Section::Activity(_) => self.snapshot.as_ref().map(Snapshot::metrics).unwrap_or_else(|| "No activity metrics captured".into()),
            Section::Audit => "Successful confirmed safety overrides recorded by this app. This is not a complete PostgreSQL audit".into(),
            Section::Server(_) => "Reader-session scope. Inspection timeout overrides are labelled; search excludes setting values".into(),
        }
    }
    pub(super) fn capture_limits(&self) -> String {
        match self.section {
            Section::Activity(_) => self.snapshot.as_ref().map(Snapshot::limits),
            Section::Server(section) => self
                .server
                .as_ref()
                .map(|snapshot| snapshot.limits(section)),
            Section::Audit => self.audit.capture.as_ref().map(|capture| capture.limits()),
        }
        .unwrap_or_default()
    }
    pub(super) fn capture_empty_label(&self) -> String {
        match self.section {
            Section::Activity(_) => self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.empty_label().to_owned()),
            Section::Server(section) => self
                .server
                .as_ref()
                .map(|snapshot| snapshot.empty_label(section)),
            Section::Audit => Some(
                self.audit
                    .capture
                    .as_ref()
                    .map_or(
                        "No local capture; Refresh works while disconnected",
                        |capture| capture.empty_label(),
                    )
                    .into(),
            ),
        }
        .unwrap_or_else(|| "No capture; Connect, then Refresh".into())
    }
    pub(super) fn capture_stale(&self) -> bool {
        match self.section {
            Section::Activity(_) => self.stale,
            Section::Server(_) => self.server_stale,
            Section::Audit => self.audit.stale,
        }
    }
    pub(super) fn settle_server(
        &mut self,
        id: u64,
        result: Result<ServerDetailsSnapshot, Arc<DataError>>,
        cx: &mut Context<Self>,
    ) {
        match self.read.settle(id) {
            Reply::Stale => return,
            Reply::Cancelled => {
                self.failure = None;
                self.status =
                    "Server read cancelled; late reply discarded and previous capture retained"
                        .into();
            }
            Reply::Current => match result {
                Ok(data) => match ServerCapture::new(data, self.budget.clone()) {
                    Ok(mut capture) => {
                        if let Err(error) = capture.set_filter(&self.filter_query, self.non_default)
                        {
                            self.status = error.into();
                            return;
                        }
                        self.server = Some(capture);
                        self.server_stale = false;
                        self.failure = None;
                        self.status =
                            "Server readings collected; values describe the inspection session"
                                .into();
                        if matches!(self.section, Section::Server(_)) {
                            self.scroll.scroll_to_item(
                                self.selected_index().unwrap_or(0),
                                gpui::ScrollStrategy::Top,
                            );
                            self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
                        }
                    }
                    Err(error) => self.status = error.into(),
                },
                Err(error) => {
                    self.status =
                        format!("Server read failed: {error:?}; previous capture retained")
                }
            },
        }
        cx.notify();
    }
    pub(super) fn apply_filter(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.filter.update(cx, |field, cx| field.try_admit(cx)) {
            self.status =
                "Settings search needs shared memory; clear another capture and retry".into();
            return;
        }
        if self
            .filter
            .update(cx, |field, cx| field.composing(window, cx))
        {
            self.status = "Finish composing before changing the settings filter".into();
            return;
        }
        let query = if matches!(action, Action::Search) {
            match self.filter.read(cx).value(cx) {
                Ok(query) => query,
                Err(error) => {
                    self.status = error.into();
                    return;
                }
            }
        } else {
            self.server
                .as_ref()
                .map_or(self.filter_query.as_str(), ServerCapture::query)
                .to_owned()
        };
        let current = self
            .server
            .as_ref()
            .map_or(self.non_default, ServerCapture::non_default);
        let non_default = if matches!(action, Action::NonDefault) {
            !current
        } else {
            current
        };
        if let Some(capture) = &mut self.server
            && let Err(error) = capture.set_filter(&query, non_default)
        {
            self.status = error.into();
            return;
        }
        self.filter_query = query;
        self.non_default = non_default;
        self.scroll.scroll_to_item(
            self.selected_index().unwrap_or(0),
            gpui::ScrollStrategy::Top,
        );
        self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.status = "Filter applied to captured settings; no database request sent".into();
    }
}

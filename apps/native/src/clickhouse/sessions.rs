//! Workspace-owned ClickHouse sessions, one per connection. Connect and
//! disconnect are explicit; a transport failure seen by any document marks the
//! session failed and leaves reconnecting to the user.
use super::session_model::SessionModel;
use crate::{controller::Host, document_view::ConnectionPhase};
use dbunk_lib::backend::clickhouse::ClickHouseSession;
use gpui::{Context, EventEmitter, Task};
use std::{collections::HashMap, sync::Arc};

/// Some connection's phase or session changed.
pub struct SessionsChanged;

pub struct ClickHouseSessions {
    host: Arc<Host>,
    model: SessionModel<ClickHouseSession>,
    /// One settling task per connection; a newer attempt replaces it.
    attempts: HashMap<String, Task<()>>,
}
impl EventEmitter<SessionsChanged> for ClickHouseSessions {}

impl ClickHouseSessions {
    pub fn new(host: Arc<Host>) -> Self {
        Self {
            host,
            model: SessionModel::default(),
            attempts: HashMap::new(),
        }
    }

    pub fn phase(&self, id: &str) -> ConnectionPhase {
        self.model.phase(id)
    }

    pub fn session(&self, id: &str) -> Option<ClickHouseSession> {
        self.model.session(id)
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SessionsChanged);
        cx.notify();
    }

    fn close(&self, sessions: impl IntoIterator<Item = ClickHouseSession>) {
        for session in sessions {
            self.host
                .runtime
                .spawn(async move { session.close().await });
        }
    }

    /// Opens a session unless one is open or opening. One bounded attempt.
    pub fn connect(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(attempt) = self.model.begin(id) else {
            return;
        };
        let backend = self.host.backend.clone();
        let target = id.to_owned();
        let open = self
            .host
            .runtime
            .spawn(async move { backend.open_clickhouse_session(target).await });
        let id = id.to_owned();
        let settle = cx.spawn({
            let id = id.clone();
            async move |this, cx| {
                let result = match open.await {
                    Ok(Ok(session)) => Ok(session),
                    Ok(Err(error)) => Err(error.message),
                    Err(_) => Err("Connect task stopped".into()),
                };
                this.update(cx, |this, cx| {
                    let stale = this.model.settle(&id, attempt, result);
                    this.close(stale);
                    this.changed(cx);
                })
                .ok();
            }
        });
        self.attempts.insert(id, settle);
        self.changed(cx);
    }

    /// Closes the connection's session; an attempt in flight is discarded
    /// when it settles.
    pub fn disconnect(&mut self, id: &str, cx: &mut Context<Self>) {
        let session = self.model.end(id);
        self.close(session);
        self.changed(cx);
    }

    /// A document's request on `used` lost the transport.
    pub fn lost(
        &mut self,
        id: &str,
        used: &ClickHouseSession,
        error: String,
        cx: &mut Context<Self>,
    ) {
        let session = self.model.lost(id, |session| session.same(used), error);
        if session.is_some() {
            self.close(session);
            self.changed(cx);
        }
    }

    /// Ends sessions `keep` rejects: deleted, edited or no longer ClickHouse.
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool, cx: &mut Context<Self>) {
        let closed = self.model.retain(keep);
        if !closed.is_empty() {
            self.close(closed);
        }
        self.changed(cx);
    }

    /// Workspace close: every session is released.
    pub fn close_all(&mut self, cx: &mut Context<Self>) {
        let sessions = self.model.drain();
        self.attempts.clear();
        self.close(sessions);
        self.changed(cx);
    }
}

//! Baseline 30-second foreground health tick for explicitly connected saved
//! connections. Probes are separate sockets: they never use, reconnect or
//! retire a document session and never record activity. A failure is shown,
//! not acted on; owned sessions settle through their own heartbeats.
use super::*;
use dbunk_lib::backend::{DevelopmentConnectionFailure, DevelopmentConnectionTest};
use std::collections::{BTreeSet, HashMap};

pub(super) const TICK: Duration = Duration::from_secs(30);

pub(super) enum Health {
    Healthy(u64),
    Failed(String),
}
#[derive(Default)]
pub(super) struct HealthState {
    results: HashMap<String, (Health, Instant)>,
    running: bool,
}
impl HealthState {
    pub(super) fn label(&self, connection: &str) -> Option<String> {
        let (health, checked) = self.results.get(connection)?;
        let age = checked.elapsed().as_secs();
        Some(match health {
            Health::Healthy(latency) => format!("Healthy · {latency} ms · checked {age}s ago"),
            Health::Failed(reason) => format!("Health check failed: {reason} · {age}s ago"),
        })
    }
    /// Age-free form for views that are refreshed only when a tick settles.
    pub(super) fn summary(&self, connection: &str) -> Option<String> {
        Some(match &self.results.get(connection)?.0 {
            Health::Healthy(latency) => format!("healthy, {latency} ms probe"),
            Health::Failed(reason) => format!("health check failed: {reason}"),
        })
    }
    /// Results for connections no longer connected are dropped, so a stale
    /// "Healthy" never outlives the session it described.
    fn settle(
        &mut self,
        results: Vec<(String, Result<DevelopmentConnectionTest, String>)>,
        connected: &BTreeSet<String>,
    ) {
        self.running = false;
        self.results.retain(|id, _| connected.contains(id));
        for (id, result) in results {
            if !connected.contains(&id) {
                continue;
            }
            let health = match result {
                Ok(DevelopmentConnectionTest::Reachable { latency_ms }) => {
                    Health::Healthy(latency_ms)
                }
                Ok(DevelopmentConnectionTest::Failed { reason }) => Health::Failed(match reason {
                    DevelopmentConnectionFailure::ConnectionLost => "connection lost".into(),
                    DevelopmentConnectionFailure::Timeout => "timed out".into(),
                    DevelopmentConnectionFailure::Authentication => "authentication".into(),
                    DevelopmentConnectionFailure::Database => "database refused".into(),
                    DevelopmentConnectionFailure::Tls(kind) => format!("TLS ({kind:?})"),
                    DevelopmentConnectionFailure::SshTunnel => "SSH route failed".into(),
                    DevelopmentConnectionFailure::SshHostKey => "SSH host key needs review".into(),
                }),
                Err(error) => Health::Failed(error),
            };
            self.results.insert(id, (health, Instant::now()));
        }
    }
}

impl Workspace {
    pub(super) fn start_health_ticks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.health_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let Ok(()) = this.update_in(cx, |this, window, cx| {
                    // Foreground only, like the baseline tick.
                    if window.is_window_active() {
                        this.check_health(cx);
                    }
                }) else {
                    return;
                };
            }
        }));
    }

    fn check_health(&mut self, cx: &mut Context<Self>) {
        if self.closing || self.health.running {
            return;
        }
        let saved: BTreeSet<&str> = self
            .connections
            .iter()
            .filter(|c| c.postgres.is_some() && c.unsupported_reason.is_none())
            .map(|c| c.id.as_str())
            .collect();
        let targets: Vec<String> = self
            .host
            .connected()
            .into_iter()
            .filter(|id| saved.contains(id.as_str()))
            .collect();
        if targets.is_empty() {
            self.health.results.clear();
            cx.notify();
            return;
        }
        self.health.running = true;
        let host = self.host.clone();
        let task = self.host.runtime.spawn(async move {
            futures_util::future::join_all(targets.into_iter().map(|id| {
                let host = host.clone();
                async move {
                    let result = host
                        .backend
                        .health_check_development_connection(id.clone())
                        .await;
                    (id, result)
                }
            }))
            .await
        });
        self.health_probe = Some(cx.spawn(async move |this, cx| {
            let results = task.await.unwrap_or_default();
            this.update(cx, |this, cx| {
                // Recompute: a connection disconnected mid-probe is dropped.
                let connected = this.host.connected();
                this.health.settle(results, &connected);
                for document in &this.documents {
                    let health = document
                        .metadata
                        .connection_id
                        .as_deref()
                        .and_then(|id| this.health.summary(id));
                    document
                        .view
                        .update(cx, |view, cx| view.set_connection_health(health, cx));
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_for_disconnected_connections_are_never_shown() {
        let mut state = HealthState {
            running: true,
            ..Default::default()
        };
        let connected = BTreeSet::from(["a".to_string(), "c".to_string()]);
        state.settle(
            vec![
                (
                    "a".into(),
                    Ok(DevelopmentConnectionTest::Reachable { latency_ms: 12 }),
                ),
                (
                    "b".into(),
                    Ok(DevelopmentConnectionTest::Reachable { latency_ms: 1 }),
                ),
                ("c".into(), Err("Credential storage is locked".into())),
            ],
            &connected,
        );
        assert!(!state.running);
        assert!(state.label("a").unwrap().starts_with("Healthy · 12 ms"));
        assert!(state.label("b").is_none());
        assert!(
            state
                .label("c")
                .unwrap()
                .starts_with("Health check failed: Credential storage is locked")
        );
        state.settle(Vec::new(), &BTreeSet::from(["c".to_string()]));
        assert!(state.label("a").is_none());
        assert!(state.label("c").is_some());
    }
}

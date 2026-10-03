//! Plan 031 step 4: engine-specific switches and the bounded Test probe for
//! MySQL, SQLite, ClickHouse and Redis. PostgreSQL keeps its staged diagnosis.
use super::engine::{Engine, Toggle};
use super::*;

impl Form {
    pub(super) fn engine_view(&mut self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let toggles: &[(Toggle, &str, &str)] = match self.engine {
            Engine::Postgres => return None,
            Engine::Sqlite => {
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(div().flex().gap_2().child(self.button(
                            "Choose database file",
                            FormAction::Pick("path"),
                            false,
                            cx,
                        )))
                        .child("The file must already exist; dbunk never creates it."),
                );
            }
            Engine::MySql => &[(Toggle::MySqlTls, "TLS: preferred", "TLS: off")],
            Engine::ClickHouse => &[(Toggle::Https, "HTTPS: on", "HTTPS: off")],
            Engine::Redis if self.toggles.redis_tls => &[
                (Toggle::RedisTls, "TLS: on", "TLS: off"),
                (
                    Toggle::RedisVerify,
                    "Verify certificate: on",
                    "Verify certificate: off",
                ),
            ],
            Engine::Redis => &[(Toggle::RedisTls, "TLS: on", "TLS: off")],
        };
        let mut row = div().flex().flex_wrap().gap_2().child("Transport");
        for (toggle, on, off) in toggles {
            let value = self.toggles.get(*toggle);
            row = row.child(self.button(
                if value { *on } else { *off },
                FormAction::EngineToggle(*toggle),
                value,
                cx,
            ));
        }
        Some(div().flex().flex_col().gap_2().child(row))
    }

    /// One explicit unsaved probe through the backend's bounded dispatch ping.
    /// Results from an older form revision are discarded.
    pub(super) fn test_engine_connection(&mut self, cx: &mut Context<Self>) {
        let Kind::Connection { id } = &self.kind else {
            return;
        };
        let id = id.clone();
        let form = match self.engine_input(cx) {
            Ok(form) => form,
            Err(error) => {
                self.message = Some(error);
                cx.notify();
                return;
            }
        };
        let password = self.value("password", cx);
        let Some(state) = self.diagnosis.as_mut() else {
            return;
        };
        state.invalidate();
        let revision = state.revision();
        let backend = self.host.backend.clone();
        let work = self.host.runtime.spawn(async move {
            backend
                .test_development_engine_connection(id, form, password)
                .await
        });
        self.busy = true;
        self.message = Some("Testing connection…".into());
        for field in &self.fields {
            field
                .editor
                .update(cx, |editor, _| editor.set_read_only(true));
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let result = work
                .await
                .unwrap_or_else(|_| Err("Connection test could not finish".into()));
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                for field in &this.fields {
                    field
                        .editor
                        .update(cx, |editor, _| editor.set_read_only(false));
                }
                let current = this
                    .diagnosis
                    .as_ref()
                    .is_some_and(|state| state.revision() == revision);
                this.message = Some(if current {
                    test_message(result)
                } else {
                    "Connection settings changed; test again".into()
                });
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn test_message(result: Result<DevelopmentConnectionTest, String>) -> String {
    match result {
        Ok(DevelopmentConnectionTest::Reachable { latency_ms }) => {
            format!("Connected in {latency_ms} ms")
        }
        Ok(DevelopmentConnectionTest::Failed { reason }) => format!(
            "Connection test failed: {}",
            match reason {
                DevelopmentConnectionFailure::ConnectionLost => "server unreachable".to_owned(),
                DevelopmentConnectionFailure::Timeout => "timed out".to_owned(),
                DevelopmentConnectionFailure::Authentication =>
                    "authentication rejected".to_owned(),
                DevelopmentConnectionFailure::Database =>
                    "server refused the connection".to_owned(),
                DevelopmentConnectionFailure::Tls(kind) => format!("TLS ({kind:?})"),
                DevelopmentConnectionFailure::SshTunnel => "SSH route failed".to_owned(),
                DevelopmentConnectionFailure::SshHostKey => {
                    "SSH host key needs review under Bastion servers".to_owned()
                }
            }
        ),
        Err(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_results_are_redacted_classes_or_local_validation_text() {
        assert_eq!(
            test_message(Ok(DevelopmentConnectionTest::Reachable { latency_ms: 7 })),
            "Connected in 7 ms"
        );
        assert_eq!(
            test_message(Ok(DevelopmentConnectionTest::Failed {
                reason: DevelopmentConnectionFailure::Authentication
            })),
            "Connection test failed: authentication rejected"
        );
        assert_eq!(
            test_message(Err("SQLite database file does not exist".into())),
            "SQLite database file does not exist"
        );
    }
}

//! A report describes one frozen form revision. Input edits discard it; the
//! control cancels on form destruction while Host retains the worker join.
use super::*;
use dbunk_lib::backend::connection_diagnosis::*;
use std::{cell::Cell, rc::Rc};

const RETAINED_BYTES: usize = 256 * 1024;
const WORKSPACE_BYTES: usize = 128 * 1024 * 1024;

struct Lease(Rc<Cell<usize>>);
impl Drop for Lease {
    fn drop(&mut self) {
        self.0.set(self.0.get().saturating_sub(RETAINED_BYTES));
    }
}

pub(super) struct State {
    budget: Rc<Cell<usize>>,
    lease: Option<Lease>,
    revision: u64,
    control: Option<ConnectionDiagnosisControl>,
    cancelled: Cell<bool>,
    report: Option<NativeDiagnosis>,
}

impl State {
    pub(super) fn new(budget: Rc<Cell<usize>>) -> Self {
        Self {
            budget,
            lease: None,
            revision: 0,
            control: None,
            cancelled: Cell::new(false),
            report: None,
        }
    }
    pub(super) fn invalidate(&mut self) {
        self.revision = self.revision.saturating_add(1);
        self.report = None;
        if let Some(control) = &self.control {
            control.cancel();
        } else {
            self.lease = None;
        }
    }
    pub(super) fn cancel(&self) {
        if let Some(control) = &self.control {
            self.cancelled.set(true);
            control.cancel();
        }
    }
    pub(super) fn running(&self) -> bool {
        self.control.is_some()
    }
    /// Current form revision; input edits advance it.
    pub(super) fn revision(&self) -> u64 {
        self.revision
    }
    fn admit(&mut self) -> Result<(), &'static str> {
        self.report = None;
        self.lease = None;
        if RETAINED_BYTES > WORKSPACE_BYTES.saturating_sub(self.budget.get()) {
            return Err(
                "Connection diagnosis needs shared memory; clear a retained result or tool",
            );
        }
        self.budget.set(self.budget.get() + RETAINED_BYTES);
        self.lease = Some(Lease(self.budget.clone()));
        Ok(())
    }
    pub(super) fn view(&self) -> Option<gpui::AnyElement> {
        let report = self.report.as_ref()?;
        let mut view = div()
            .id("connection-diagnosis")
            .role(Role::List)
            .aria_label("Connection diagnosis")
            .flex()
            .flex_col()
            .gap(px(3.))
            .p(px(8.))
            .rounded(px(5.))
            .border_1()
            .border_color(crate::style::line())
            .bg(crate::style::panel())
            .font_family(crate::style::MONO)
            .text_size(px(crate::style::FONT_SMALL))
            .text_color(crate::style::dim());
        for (index, stage) in report.stages.iter().enumerate() {
            let label = stage_line(stage);
            view = view.child(
                div()
                    .id(("diagnosis-stage", index))
                    .role(Role::ListItem)
                    .aria_label(label.clone())
                    .child(label),
            );
        }
        for (index, warning) in report.warnings.iter().enumerate() {
            let label = warning_text(*warning);
            view = view.child(
                div()
                    .id(("diagnosis-warning", index))
                    .role(Role::ListItem)
                    .aria_label(label)
                    .child(label),
            );
        }
        Some(
            view.child(
                div()
                    .id("diagnosis-channel-binding-limitation")
                    .role(Role::ListItem)
                    .aria_label(CHANNEL_BINDING_LIMITATION)
                    .child(CHANNEL_BINDING_LIMITATION),
            )
            .into_any_element(),
        )
    }
}

impl Form {
    pub(super) fn test_connection(&mut self, cx: &mut Context<Self>) {
        let Kind::Connection { id } = &self.kind else {
            return;
        };
        if self.engine != super::engine::Engine::Postgres {
            self.test_engine_connection(cx);
            return;
        }
        let id = id.clone();
        let form = match self.connection_input(cx) {
            Ok(form) => form,
            Err(error) => {
                self.fail(error);
                cx.notify();
                return;
            }
        };
        let password = self.value("password", cx);
        let Some(state) = self.diagnosis.as_mut() else {
            return;
        };
        if let Err(error) = state.admit() {
            self.fail(error);
            cx.notify();
            return;
        }
        let (control, receive) = match self.host.diagnose_connection(id, form, password) {
            Ok(work) => work,
            Err(error) => {
                state.lease = None;
                self.fail(error);
                cx.notify();
                return;
            }
        };
        let revision = state.revision;
        state.cancelled.set(false);
        state.control = Some(control);
        self.busy = true;
        self.message = None;
        for field in &self.fields {
            field
                .editor
                .update(cx, |editor, _| editor.set_read_only(true));
        }
        self.task = Some(cx.spawn(async move |this, cx| {
            let delivery = receive.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                for field in &this.fields {
                    field
                        .editor
                        .update(cx, |editor, _| editor.set_read_only(false));
                }
                let Some(state) = this.diagnosis.as_mut() else {
                    return;
                };
                state.control = None;
                let result = if state.cancelled.get() {
                    Err("Connection diagnosis cancelled".to_string())
                } else {
                    delivery
                        .map_err(|_| "Connection diagnosis could not finish".to_string())
                        .and_then(|delivery| delivery.result)
                };
                if state.revision != revision {
                    state.lease = None;
                    this.note("Connection settings changed; test again");
                } else {
                    match result {
                        Ok(report) if report.checked_heap_bytes().is_some() => {
                            let outcome = match &report.outcome {
                                NativeDiagnosisOutcome::Reachable { latency_ms } => {
                                    Ok(format!("Connected in {latency_ms} ms"))
                                }
                                NativeDiagnosisOutcome::Failed { stage } => {
                                    Err(format!("Connection test failed at {}", stage_name(*stage)))
                                }
                            };
                            state.report = Some(report);
                            match outcome {
                                Ok(text) => this.say(text, Tone::Success),
                                Err(text) => this.fail(text),
                            }
                        }
                        Ok(_) => {
                            state.lease = None;
                            this.fail("Connection diagnosis exceeded its display limit");
                        }
                        Err(error) => {
                            state.lease = None;
                            this.fail(error);
                        }
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

fn stage_name(stage: NativeStageKind) -> &'static str {
    match stage {
        NativeStageKind::Tunnel => "Tunnel",
        NativeStageKind::Dns => "DNS",
        NativeStageKind::Tcp => "TCP",
        NativeStageKind::Tls => "TLS",
        NativeStageKind::Authentication => "Authentication",
        NativeStageKind::Database => "Database",
    }
}

fn stage_line(stage: &NativeDiagnosisStage) -> String {
    let result = match &stage.result {
        NativeStageResult::Skipped { reason } => format!(
            "Skipped: {}",
            match reason {
                NativeSkipReason::NoTunnel => "direct connection",
                NativeSkipReason::TlsDisabled => "TLS disabled",
                NativeSkipReason::BlockedByEarlierFailure => "earlier stage failed",
                NativeSkipReason::NotApplicable => "not applicable",
            }
        ),
        NativeStageResult::Failed {
            elapsed_ms,
            message,
            ..
        } => format!("Failed ({elapsed_ms} ms): {message}"),
        NativeStageResult::Passed { elapsed_ms, detail } => {
            let detail = detail.as_ref().map(detail_text).unwrap_or_default();
            format!("Passed ({elapsed_ms} ms) {detail}")
        }
    };
    format!("{}: {result}", stage_name(stage.stage))
}

fn detail_text(detail: &NativeStageDetail) -> String {
    match detail {
        NativeStageDetail::Tunnel { local_endpoint } => local_endpoint.clone(),
        NativeStageDetail::Dns { addresses } => addresses.join(", "),
        NativeStageDetail::Database { server_version } => format!("PostgreSQL {server_version}"),
        NativeStageDetail::Tls {
            encrypted,
            protocol,
            cipher,
            certificate_verified,
            hostname_verified,
            client_certificate_presented,
            pool_hostname_verification_ca_only,
        } => {
            if !encrypted {
                return "Plaintext".into();
            }
            let yes = |value: bool| if value { "yes" } else { "no" };
            format!(
                "Encrypted; protocol {}; cipher {}; certificate verified {}; hostname verified {}; client certificate presented {}{}",
                protocol.as_deref().unwrap_or("unavailable"),
                cipher.as_deref().unwrap_or("unavailable"),
                yes(*certificate_verified),
                yes(*hostname_verified),
                if *client_certificate_presented {
                    "observed"
                } else {
                    "not observed"
                },
                if *pool_hostname_verification_ca_only {
                    "; metadata pool verifies CA only"
                } else {
                    ""
                }
            )
        }
    }
}

fn warning_text(warning: NativeDiagnosisWarning) -> &'static str {
    match warning {
        NativeDiagnosisWarning::NotEncrypted => "Warning: connection is not encrypted.",
        NativeDiagnosisWarning::PoolHostnameVerificationCaOnly => {
            "Warning: metadata connections verify the certificate authority only."
        }
        NativeDiagnosisWarning::ProductionWithoutVerification => {
            "Warning: production connection does not verify the server certificate."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_admission_and_invalidation_release_only_their_allowance() {
        let budget = Rc::new(Cell::new(WORKSPACE_BYTES - RETAINED_BYTES));
        let mut first = State::new(budget.clone());
        let mut second = State::new(budget.clone());
        first.admit().unwrap();
        assert_eq!(budget.get(), WORKSPACE_BYTES);
        assert!(second.admit().is_err());
        first.invalidate();
        assert_eq!(budget.get(), WORKSPACE_BYTES - RETAINED_BYTES);
        second.admit().unwrap();
        drop(second);
        assert_eq!(budget.get(), WORKSPACE_BYTES - RETAINED_BYTES);
    }
    #[test]
    fn plaintext_and_blocked_stages_do_not_claim_verified_encryption_or_success() {
        assert_eq!(
            detail_text(&NativeStageDetail::Tls {
                encrypted: false,
                protocol: None,
                cipher: None,
                certificate_verified: false,
                hostname_verified: false,
                client_certificate_presented: false,
                pool_hostname_verification_ca_only: false
            }),
            "Plaintext"
        );
        assert_eq!(
            stage_line(&NativeDiagnosisStage {
                stage: NativeStageKind::Database,
                result: NativeStageResult::Skipped {
                    reason: NativeSkipReason::BlockedByEarlierFailure
                }
            }),
            "Database: Skipped: earlier stage failed"
        );
    }
}

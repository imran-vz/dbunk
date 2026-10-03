//! One unsaved, direct PostgreSQL probe. Cancellation belongs to the caller's
//! form; task and socket ownership remain in the backend through teardown.
use super::{Backend, DevelopmentPostgresConnection};
use crate::credentials;
use tokio::sync::watch;

pub use crate::diagnosis::native::{
    NativeDiagnosis, NativeDiagnosisOutcome, NativeDiagnosisStage, NativeDiagnosisWarning,
    NativeFailureKind, NativeSkipReason, NativeStageDetail, NativeStageKind, NativeStageResult,
    CHANNEL_BINDING_LIMITATION, MAX_DIAGNOSIS_BYTES,
};

/// Dropping the control cancels this attempt, including while it is queued.
/// It cannot cancel another form's attempt and is deliberately not Clone.
pub struct ConnectionDiagnosisControl(watch::Sender<bool>);

pub struct ConnectionDiagnosisRequest {
    profile: String,
    cancellation: watch::Receiver<bool>,
}

impl ConnectionDiagnosisControl {
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
}

impl Drop for ConnectionDiagnosisControl {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Backend {
    pub fn connection_diagnosis_control(
        &self,
    ) -> Result<(ConnectionDiagnosisControl, ConnectionDiagnosisRequest), String> {
        let authority = self.development()?;
        let (send, cancellation) = watch::channel(false);
        Ok((
            ConnectionDiagnosisControl(send),
            ConnectionDiagnosisRequest {
                profile: authority.profile_id.clone(),
                cancellation,
            },
        ))
    }

    pub async fn diagnose_native_connection(
        &self,
        request: ConnectionDiagnosisRequest,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
    ) -> Result<NativeDiagnosis, String> {
        let authority = self.development()?;
        if request.profile != authority.profile_id {
            return Err("Connection diagnosis belongs to another profile".into());
        }
        let drivers = self.0.tasks.child();
        self.development_call(move |state| async move {
            Ok(async {
                let mut cancellation = request.cancellation;
                if *cancellation.borrow() || cancellation.has_changed().is_err() {
                    return Err("Connection diagnosis cancelled".into());
                }
                let _guard = tokio::select! {
                    biased;
                    _ = cancellation.changed() => return Err("Connection diagnosis cancelled".into()),
                    guard = credentials::mutation_guard(&state.credentials) => guard,
                };
                let connection = super::development::connections::prepare_probe(
                    &state, &authority, id, form, password,
                )
                .await?;
                let crate::StoredConnection::PostgreSQL(pg) = connection else {
                    return Err("PostgreSQL connection required".into());
                };
                // Keep the same admission and credential snapshot through the
                // bounded probe; a later save/reset cannot overtake it.
                crate::diagnosis::native::run(&pg, &drivers, cancellation).await
            }
            .await)
        })
        .await
        .map_err(|_| "Native backend is closing".to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelling_or_dropping_a_control_affects_only_its_attempt() {
        let (first, first_rx) = watch::channel(false);
        let (second, second_rx) = watch::channel(false);
        let first = ConnectionDiagnosisControl(first);
        let second = ConnectionDiagnosisControl(second);
        first.cancel();
        assert!(*first_rx.borrow());
        assert!(!*second_rx.borrow());
        drop(second);
        assert!(*second_rx.borrow());
        assert!(second_rx.has_changed().is_err());
    }
}

//! The host retains the join even when the form disappears. Queued report
//! payloads hold the same delivery allowance used by result and tool workers.
use super::{Host, Worker};
use crate::mailbox::BytePermit;
use dbunk_lib::backend::{
    DevelopmentPostgresConnection,
    connection_diagnosis::{ConnectionDiagnosisControl, NativeDiagnosis},
};
use futures_util::FutureExt;
use std::sync::atomic::Ordering;
use tokio::sync::oneshot;

const DELIVERY_BYTES: usize = 64 * 1024 + 4096;

pub struct DiagnosisDelivery {
    pub result: Result<NativeDiagnosis, String>,
    _permit: BytePermit,
}

impl Host {
    pub fn diagnose_connection(
        &self,
        id: Option<String>,
        form: DevelopmentPostgresConnection,
        password: String,
    ) -> Result<
        (
            ConnectionDiagnosisControl,
            oneshot::Receiver<DiagnosisDelivery>,
        ),
        String,
    > {
        // Match shutdown's lock order; the worker is registered before its
        // submission lock can be released to shutdown.
        let mut sessions = self.sessions.lock().unwrap();
        let _submission = self.submission.lock().unwrap();
        if self.closing.load(Ordering::Acquire) {
            return Err("Application is closing".into());
        }
        let mut failure = None;
        sessions
            .workers
            .retain(|worker| match worker.join.clone().now_or_never() {
                Some(Err(error)) => {
                    failure = Some(error.to_string());
                    false
                }
                Some(Ok(())) => false,
                None => true,
            });
        if let Some(error) = failure {
            sessions.failure.get_or_insert(error.clone());
            return Err(error);
        }
        if sessions.workers.len() >= 16 {
            return Err("Wait for earlier connection probes to finish closing".into());
        }
        let permit = self
            .workspace
            .as_ref()
            .ok_or("A workspace host is required")?
            .queue_budget()
            .reserve(DELIVERY_BYTES)
            .ok_or("Connection diagnosis needs delivery memory; wait for pending results")?;
        let (control, request) = self.backend.connection_diagnosis_control()?;
        let backend = self.backend.clone();
        let (send, receive) = oneshot::channel();
        let worker = Worker::new(self.runtime.spawn(async move {
            let result = backend
                .diagnose_native_connection(request, id, form, password)
                .await
                .map_err(|mut error| {
                    // Storage errors are not retained as unbounded UI strings.
                    if error.len() > 4096 {
                        let mut end = 4096;
                        while !error.is_char_boundary(end) {
                            end -= 1;
                        }
                        error.truncate(end);
                    }
                    error.shrink_to_fit();
                    error
                });
            let _ = send.send(DiagnosisDelivery {
                result,
                _permit: permit,
            });
            Ok(())
        }));
        sessions.workers.push(worker);
        Ok((control, receive))
    }
}

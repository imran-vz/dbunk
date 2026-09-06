use super::*;
use crate::postgres::schema_compare::values::{encode, EncodedPage, ValueRequest};
use serde::Serialize;

impl CompareManager {
    /// Serialize under the result lock, then retain the lease before handing
    /// the body to IPC. Release/teardown cannot race an unowned result borrow.
    pub(crate) fn read(
        &self,
        session: &str,
        transport: &str,
        response_id: &str,
        request: &ResultRequest,
        read: ReadRequest,
        send: impl FnOnce(String),
    ) -> Result<(), CompareError> {
        let mut s = self.inner.lock().unwrap();
        validate_transport(&s, session, transport)?;
        let now = Instant::now();
        prune(&mut s, now);
        let entry = s
            .jobs
            .iter()
            .find(|e| e.status.job_id == request.identity.job_id && !e.invalidated)
            .ok_or(CompareError::Unavailable)?;
        let result = entry.result.as_ref().ok_or(CompareError::Unavailable)?;
        result.validate_read(&request.identity, now)?;
        let metadata = result.metadata();
        if metadata.identity != request.identity
            || metadata.source.endpoint != request.source
            || metadata.target.endpoint != request.target
        {
            return Err(CompareError::Unavailable);
        }
        let page = match read {
            ReadRequest::Objects { offset } => {
                result.object_page(&request.identity, response_id, offset, now)?
            }
            ReadRequest::Fields { object, offset } => {
                result.field_page(&request.identity, &object, response_id, offset, now)?
            }
            ReadRequest::Value { value, offset } => result.value_chunk(
                &ValueRequest {
                    identity: request.identity.clone(),
                    value,
                    offset,
                },
                response_id,
                now,
            )?,
            ReadRequest::Eligibility { object, side } => self.encode_detail(
                response_id,
                &request.identity,
                &EligibilityDetail {
                    object: &object,
                    side,
                    eligibility: result.relation_eligibility(
                        &request.identity,
                        &object,
                        side,
                        now,
                    )?,
                },
            )?,
            ReadRequest::Metadata => self.encode_detail(
                response_id,
                &request.identity,
                &MetadataDetail {
                    metadata,
                    kind: result.kind(),
                    object_count: result.object_count(),
                    source_excluded_counts: result.excluded_counts(Side::Source),
                    target_excluded_counts: result.excluded_counts(Side::Target),
                },
            )?,
        };
        s.responses.handoff(session, page, send)
    }
    fn encode_detail(
        &self,
        response_id: &str,
        identity: &ResultIdentity,
        detail: &impl Serialize,
    ) -> Result<EncodedPage, CompareError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Detail<'a, T> {
            response_id: &'a str,
            identity: &'a ResultIdentity,
            detail: &'a T,
        }
        encode(
            &Detail {
                response_id,
                identity,
                detail,
            },
            response_id,
            self.budget.serializer()?,
        )
    }
    pub(crate) fn acknowledge(
        &self,
        session: &str,
        transport: &str,
        response_id: &str,
    ) -> Result<(), CompareError> {
        let mut s = self.inner.lock().unwrap();
        validate_transport(&s, session, transport)?;
        s.responses.acknowledge(session, response_id)
    }

    pub(crate) fn transport(&self, window: &str) -> Result<String, CompareError> {
        self.inner
            .lock()
            .unwrap()
            .transports
            .iter()
            .find(|t| t.window == window)
            .map(|t| t.token.clone())
            .ok_or(CompareError::Unavailable)
    }

    /// Wry 0.55.1 emits Started at document commit: WK didCommitNavigation,
    /// WebKitGTK LoadEvent::Committed, or WebView2 ContentLoading. These replace
    /// the main document; provisional starts and same-document navigation do not.
    /// Finished can also report a failed/cancelled navigation with the old document
    /// still alive, so it must never retire replies or rotate its token.
    /// See ADR 0030 for the platform contracts; recheck them when upgrading Wry.
    pub(crate) fn transport_page_load(&self, window: &str, event: tauri::webview::PageLoadEvent) {
        if !matches!(event, tauri::webview::PageLoadEvent::Started) {
            return;
        }
        let mut s = self.inner.lock().unwrap();
        s.responses.transport_destroyed(window);
        if let Some(t) = s.transports.iter_mut().find(|t| t.window == window) {
            t.token = uuid::Uuid::new_v4().to_string();
        } else if !window.is_empty() && window.len() <= 128 && s.transports.len() < MAX_TRANSPORTS {
            s.transports.push(DocumentTransport {
                window: window.into(),
                token: uuid::Uuid::new_v4().to_string(),
            });
        }
    }
    pub(crate) fn transport_destroyed(&self, session: &str) {
        let mut s = self.inner.lock().unwrap();
        s.responses.transport_destroyed(session);
        s.transports.retain(|t| t.window != session);
    }
}

fn validate_transport(s: &State, window: &str, token: &str) -> Result<(), CompareError> {
    if s.transports
        .iter()
        .any(|t| t.window == window && t.token == token)
    {
        Ok(())
    } else {
        Err(CompareError::Unavailable)
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataDetail<'a> {
    metadata: &'a ComparisonMetadata,
    kind: super::super::normalize::DifferenceKind,
    object_count: usize,
    source_excluded_counts: &'a [super::super::capture::ExcludedCount],
    target_excluded_counts: &'a [super::super::capture::ExcludedCount],
}
#[derive(Serialize)]
struct EligibilityDetail<'a> {
    object: &'a RelationIdentity,
    side: Side,
    eligibility: &'a Eligibility,
}

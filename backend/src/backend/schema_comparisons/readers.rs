use super::*;
use crate::postgres::schema_compare::values::EncodedPage;
use serde::Deserialize;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use tokio::sync::{OwnedRwLockReadGuard, RwLock};

pub(super) struct ReaderLease {
    id: uuid::Uuid,
    window: String,
    tab: String,
    request: ResultRequest,
    closed: AtomicBool,
    admission: Arc<RwLock<()>>,
    _reservation: crate::postgres::schema_compare::budget::Reservation,
}
#[derive(Default)]
pub(super) struct Readers {
    entries: Mutex<Vec<Arc<ReaderLease>>>,
}
impl Readers {
    fn register(
        &self,
        window: String,
        tab: String,
        request: ResultRequest,
        reservation: crate::postgres::schema_compare::budget::Reservation,
    ) -> Result<Arc<ReaderLease>, CompareError> {
        let mut entries = self.entries.lock().unwrap();
        entries.retain(|entry| {
            !entry.closed.load(Ordering::Acquire) || entry.admission.try_write().is_err()
        });
        if entries.len() >= MAX_COMPARISON_READERS
            || entries
                .iter()
                .any(|entry| entry.window == window && entry.tab == tab)
        {
            return Err(CompareError::Busy);
        }
        let lease = Arc::new(ReaderLease {
            id: uuid::Uuid::new_v4(),
            window,
            tab,
            request,
            closed: AtomicBool::new(false),
            admission: Arc::new(RwLock::new(())),
            _reservation: reservation,
        });
        entries.push(lease.clone());
        Ok(lease)
    }
    pub(super) fn close_all(&self) {
        for entry in self.entries.lock().unwrap().iter() {
            entry.closed.store(true, Ordering::Release);
        }
    }
    async fn finish(
        &self,
        lease: &Arc<ReaderLease>,
        deadline: tokio::time::Instant,
    ) -> Result<(), CompareError> {
        lease.closed.store(true, Ordering::Release);
        let _joined = tokio::time::timeout_at(deadline, lease.admission.write())
            .await
            .map_err(|_| CompareError::DeadlineExceeded)?;
        self.entries
            .lock()
            .unwrap()
            .retain(|entry| entry.id != lease.id);
        Ok(())
    }
    pub(super) async fn drain_until(
        &self,
        deadline: tokio::time::Instant,
    ) -> Result<(), CompareError> {
        self.close_all();
        let entries = self.entries.lock().unwrap().clone();
        for entry in entries {
            self.finish(&entry, deadline).await?;
        }
        Ok(())
    }
}
struct Handle {
    owner: Weak<Inner>,
    lease: Arc<ReaderLease>,
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.lease.closed.store(true, Ordering::Release);
    }
}
#[derive(Clone)]
pub struct SchemaComparisonReader(Arc<Handle>);
impl std::fmt::Debug for SchemaComparisonReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaComparisonReader")
            .field("generation", &self.0.lease.id)
            .finish_non_exhaustive()
    }
}
impl SchemaComparisonReader {
    pub fn request(&self) -> &ResultRequest {
        &self.0.lease.request
    }
    pub fn belongs_to(&self, backend: &Backend) -> bool {
        Weak::ptr_eq(&self.0.owner, &Arc::downgrade(&backend.0))
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        let l = &self.0.lease;
        bounds::request_bytes(&l.request)?
            .checked_add(l.window.capacity())?
            .checked_add(l.tab.capacity())?
            .checked_add(std::mem::size_of::<Handle>() + std::mem::size_of::<ReaderLease>() + 128)
    }
}
/// Owns both the serializer reservation and this exact document read permit.
/// Dropping a queued/stale response destroys its payload before freeing capacity;
/// no timer, job release or reader close acknowledges somebody else's reply.
pub struct SchemaComparisonResponse {
    owner: Weak<Inner>,
    lease: Arc<ReaderLease>,
    data: Option<SchemaComparisonPage>,
    encoded: EncodedPage,
    _permit: OwnedRwLockReadGuard<()>,
}
impl std::fmt::Debug for SchemaComparisonResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaComparisonResponse")
            .field("generation", &self.lease.id)
            .finish_non_exhaustive()
    }
}
impl SchemaComparisonResponse {
    pub fn data(&self) -> &SchemaComparisonPage {
        self.data.as_ref().expect("unconsumed response")
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.data()
            .checked_heap_bytes()?
            .checked_add(self.encoded.retained_bytes())?
            .checked_add(std::mem::size_of::<Self>())
            .filter(|n| *n <= COMPARISON_RESPONSE_RESERVATION)
    }
    pub fn into_page(mut self) -> Result<SchemaComparisonPage, CompareError> {
        if self.lease.closed.load(Ordering::Acquire) {
            return Err(CompareError::Unavailable);
        }
        let owner = self.owner.upgrade().ok_or(CompareError::Unavailable)?;
        if owner.closing.load(Ordering::Acquire) {
            return Err(CompareError::Unavailable);
        }
        owner
            .state
            .pg_schema_compare
            .validate_result(&self.lease.request)?;
        Ok(self.data.take().expect("unconsumed response"))
    }
}
impl Backend {
    pub async fn open_schema_comparison_reader(
        &self,
        window: &str,
        tab: &str,
        request: ResultRequest,
    ) -> Result<SchemaComparisonReader, CompareError> {
        if [window, tab]
            .iter()
            .any(|id| id.is_empty() || id.len() > 128)
            || bounds::request_bytes(&request).is_none()
        {
            return Err(CompareError::InvalidRequest);
        }
        let reservation = self.0.state.pg_schema_compare.reader_reservation()?;
        let (window, tab) = (window.to_owned(), tab.to_owned());
        let inner = self.0.clone();
        self.call_with_admission(self.0.data_admission.clone(), move |state| async move {
            let _gate = inner.development_gate.lock().await;
            let result = async {
                if inner.closing.load(Ordering::Acquire) {
                    return Err(CompareError::Unavailable);
                }
                for endpoint in [&request.source, &request.target] {
                    super::super::admit_postgres_connection(
                        &state,
                        inner.development.as_deref(),
                        &endpoint.connection_id,
                    )
                    .await
                    .map_err(|_| CompareError::Unavailable)?;
                }
                state.pg_schema_compare.validate_result(&request)?;
                let lease =
                    inner
                        .schema_comparisons
                        .readers
                        .register(window, tab, request, reservation)?;
                Ok(SchemaComparisonReader(Arc::new(Handle {
                    owner: Arc::downgrade(&inner),
                    lease,
                })))
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| CompareError::Unavailable)?
    }
    pub async fn read_schema_comparison(
        &self,
        reader: &SchemaComparisonReader,
        read: ReadRequest,
    ) -> Result<SchemaComparisonResponse, CompareError> {
        if !reader.belongs_to(self) || bounds::read_bytes(&read).is_none() {
            return Err(CompareError::InvalidRequest);
        }
        let lease = reader.0.lease.clone();
        let inner = self.0.clone();
        self.call_with_admission(self.0.data_admission.clone(), move |state| async move {
            let result = async {
                if lease.closed.load(Ordering::Acquire) || inner.closing.load(Ordering::Acquire) {
                    return Err(CompareError::Unavailable);
                }
                let permit = lease.admission.clone().read_owned().await;
                if lease.closed.load(Ordering::Acquire) {
                    return Err(CompareError::Unavailable);
                }
                let response_id = uuid::Uuid::new_v4().to_string();
                let encoded = state.pg_schema_compare.read_owned(
                    &response_id,
                    &lease.request,
                    read.clone(),
                )?;
                let data = decode(&encoded, response_id, lease.request.clone(), read)?;
                let reply = SchemaComparisonResponse {
                    owner: Arc::downgrade(&inner),
                    lease,
                    data: Some(data),
                    encoded,
                    _permit: permit,
                };
                reply
                    .checked_heap_bytes()
                    .ok_or(CompareError::LimitExceeded {
                        limit: Limit::Allocation,
                    })?;
                Ok(reply)
            }
            .await;
            Ok(result)
        })
        .await
        .map_err(|_| CompareError::Unavailable)?
    }
    pub async fn close_schema_comparison_reader(
        &self,
        reader: &SchemaComparisonReader,
    ) -> Result<(), CompareError> {
        if !reader.belongs_to(self) {
            return Err(CompareError::Unavailable);
        }
        self.0
            .schema_comparisons
            .readers
            .finish(
                &reader.0.lease,
                tokio::time::Instant::now() + std::time::Duration::from_secs(5),
            )
            .await
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    response_id: String,
    identity: ResultIdentity,
    offset: u32,
    next_offset: Option<u32>,
    items: Vec<T>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Detail<T> {
    response_id: String,
    identity: ResultIdentity,
    detail: T,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    metadata: ComparisonMetadata,
    kind: DifferenceKind,
    object_count: usize,
    source_excluded_counts: Vec<ExcludedCount>,
    target_excluded_counts: Vec<ExcludedCount>,
}
#[derive(Deserialize)]
struct Eligible {
    object: RelationIdentity,
    side: Side,
    eligibility: Eligibility,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chunk {
    response_id: String,
    identity: ResultIdentity,
    value: ValueRef,
    offset: u32,
    text: String,
    next_offset: u32,
    complete: bool,
}
fn parse<T: serde::de::DeserializeOwned>(page: &EncodedPage) -> Result<T, CompareError> {
    serde_json::from_str(page.as_str()).map_err(|_| CompareError::Unavailable)
}
fn decode(
    encoded: &EncodedPage,
    response_id: String,
    request: ResultRequest,
    read: ReadRequest,
) -> Result<SchemaComparisonPage, CompareError> {
    let (seen_id, seen_identity, reply) = match &read {
        ReadRequest::Metadata => {
            let p: Detail<Metadata> = parse(encoded)?;
            (
                p.response_id,
                p.identity,
                CompareReply::Metadata {
                    metadata: p.detail.metadata,
                    kind: p.detail.kind,
                    object_count: p.detail.object_count,
                    source_excluded_counts: p.detail.source_excluded_counts,
                    target_excluded_counts: p.detail.target_excluded_counts,
                },
            )
        }
        ReadRequest::Objects { .. } => {
            let p: Page<ObjectSummary> = parse(encoded)?;
            (
                p.response_id,
                p.identity,
                CompareReply::Objects {
                    offset: p.offset,
                    next_offset: p.next_offset,
                    items: p.items,
                },
            )
        }
        ReadRequest::Fields { object, .. } => {
            let p: Page<FieldSummary> = parse(encoded)?;
            (
                p.response_id,
                p.identity,
                CompareReply::Fields {
                    object: object.clone(),
                    offset: p.offset,
                    next_offset: p.next_offset,
                    items: p.items,
                },
            )
        }
        ReadRequest::Eligibility { .. } => {
            let p: Detail<Eligible> = parse(encoded)?;
            (
                p.response_id,
                p.identity,
                CompareReply::Eligibility {
                    object: p.detail.object,
                    side: p.detail.side,
                    eligibility: p.detail.eligibility,
                },
            )
        }
        ReadRequest::Value { .. } => {
            let p: Chunk = parse(encoded)?;
            (
                p.response_id,
                p.identity,
                CompareReply::Value {
                    value: p.value,
                    offset: p.offset,
                    text: p.text,
                    next_offset: p.next_offset,
                    complete: p.complete,
                },
            )
        }
    };
    if seen_id != response_id || seen_identity != request.identity {
        return Err(CompareError::Unavailable);
    }
    let result = SchemaComparisonPage {
        response_id,
        request,
        read,
        reply,
    };
    result
        .checked_heap_bytes()
        .ok_or(CompareError::LimitExceeded {
            limit: Limit::Allocation,
        })?;
    Ok(result)
}

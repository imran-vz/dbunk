use super::*;
use serde::{Deserialize, Serialize};

pub const MAX_COMPARISON_LIST_BYTES: usize = 64 * 1024;
pub const MAX_COMPARISON_PAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_COMPARISON_ENCODED_BYTES: usize = 1024 * 1024;
pub const COMPARISON_RESPONSE_RESERVATION: usize = 8 * 1024 * 1024;
pub const MAX_COMPARISON_READERS: usize = 16;

#[derive(Clone, Debug)]
pub struct SchemaComparisonStart(pub(super) StartRequest);
impl SchemaComparisonStart {
    pub fn new(source: Endpoint, target: Endpoint) -> Result<Self, CompareError> {
        let value = Self(StartRequest {
            request_id: format!(
                "{}:{}",
                chrono::Utc::now().timestamp_millis(),
                uuid::Uuid::new_v4()
            ),
            source,
            target,
        });
        value.0.validate()?;
        value
            .checked_heap_bytes()
            .ok_or(CompareError::InvalidRequest)?;
        Ok(value)
    }
    pub fn request_id(&self) -> &str {
        &self.0.request_id
    }
    pub fn source(&self) -> &Endpoint {
        &self.0.source
    }
    pub fn target(&self) -> &Endpoint {
        &self.0.target
    }
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        self.0.validate().ok()?;
        let n = std::mem::size_of::<Self>()
            .checked_add(self.0.request_id.capacity())?
            .checked_add(endpoint_bytes(&self.0.source)?)?
            .checked_add(endpoint_bytes(&self.0.target)?)?;
        (n <= 4096).then_some(n)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SchemaComparisonList {
    pub jobs: Vec<Status>,
}
impl SchemaComparisonList {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        if self.jobs.len() > 4 {
            return None;
        }
        let mut n = std::mem::size_of::<Self>().checked_add(
            self.jobs
                .capacity()
                .checked_mul(std::mem::size_of::<Status>())?,
        )?;
        for (i, row) in self.jobs.iter().enumerate() {
            if self.jobs[..i]
                .iter()
                .any(|prior| prior.job_id == row.job_id)
            {
                return None;
            }
            n = n.checked_add(status_bytes(row)?)?;
        }
        (n <= MAX_COMPARISON_LIST_BYTES).then_some(n)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSummary {
    #[serde(flatten)]
    pub difference: SummaryDifference<RelationIdentity>,
    pub field_count: u32,
    pub changed_fields: u32,
    pub incomparable_fields: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldSummary {
    pub path: FieldPath,
    #[serde(flatten)]
    pub difference: SummaryDifference<ValueRef>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "replyKind", rename_all = "camelCase")]
// One reply per admitted response; its fixed envelope is included in capacity
// accounting. Keeping metadata inline avoids a separate unmeasured allocation.
#[allow(clippy::large_enum_variant)]
pub enum CompareReply {
    Metadata {
        metadata: ComparisonMetadata,
        kind: DifferenceKind,
        object_count: usize,
        source_excluded_counts: Vec<ExcludedCount>,
        target_excluded_counts: Vec<ExcludedCount>,
    },
    Objects {
        offset: u32,
        next_offset: Option<u32>,
        items: Vec<ObjectSummary>,
    },
    Fields {
        object: RelationIdentity,
        offset: u32,
        next_offset: Option<u32>,
        items: Vec<FieldSummary>,
    },
    Eligibility {
        object: RelationIdentity,
        side: Side,
        eligibility: Eligibility,
    },
    Value {
        value: ValueRef,
        offset: u32,
        text: String,
        next_offset: u32,
        complete: bool,
    },
}
/// Owned decoded page. The host must reserve retained capacity before consuming
/// the response lease; this payload itself does not retain a serializer slot.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaComparisonPage {
    pub response_id: String,
    pub request: ResultRequest,
    pub read: ReadRequest,
    pub reply: CompareReply,
}
impl std::fmt::Debug for SchemaComparisonPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SchemaComparisonPage")
            .field("response_id", &self.response_id)
            .field("identity", &self.request.identity)
            .finish_non_exhaustive()
    }
}
impl SchemaComparisonPage {
    pub fn checked_heap_bytes(&self) -> Option<usize> {
        super::bounds::page_bytes(self)
    }
}
pub(super) fn endpoint_bytes(value: &Endpoint) -> Option<usize> {
    if value.connection_id.is_empty()
        || value.connection_id.len() > 128
        || value.schema.is_empty()
        || value.schema.len() > 63
        || value.schema.contains('\0')
    {
        return None;
    }
    let n = value
        .connection_id
        .capacity()
        .checked_add(value.schema.capacity())?;
    (n <= 512).then_some(n)
}
pub(super) fn status_bytes(value: &Status) -> Option<usize> {
    if value.job_id.is_empty()
        || value.job_id.len() > 128
        || value.request_id.is_empty()
        || value.request_id.len() > 128
    {
        return None;
    }
    let mut n = value
        .job_id
        .capacity()
        .checked_add(value.request_id.capacity())?
        .checked_add(endpoint_bytes(&value.source)?)?
        .checked_add(endpoint_bytes(&value.target)?)?;
    match &value.state {
        StatusState::Completed { result_id } => {
            if result_id.len() > 128 {
                return None;
            }
            n = n.checked_add(result_id.capacity())?;
        }
        StatusState::Failed {
            failure: CompareError::UnsupportedVersion { version, .. },
        } => {
            if version.len() > 256 {
                return None;
            }
            n = n.checked_add(version.capacity())?;
        }
        _ => {}
    }
    (n <= 4096).then_some(n)
}

//! Deterministic, immutable structural results. Capture errors cannot be passed
//! as empty endpoints; no manager, SQL execution or command registration lives here.
mod build;
#[cfg(test)]
mod tests;

use super::{
    budget::*,
    capture::ExcludedCount,
    normalize::DifferenceKind,
    pages::{self, FieldSummary, ObjectSummary, ObservedSides, SummaryDifference},
    protocol::*,
    values::{EncodedPage, ValueRef, ValueRequest, Values, RESULT_TTL},
};
use std::{mem::size_of, ops::Range, time::Instant};

pub use build::compare;

struct StoredField {
    path: FieldPath,
    difference: SummaryDifference<ValueRef>,
    _reservation: Reservation,
}

struct StoredObject {
    difference: SummaryDifference<RelationIdentity>,
    source_eligibility: Option<Eligibility>,
    target_eligibility: Option<Eligibility>,
    fields: Range<usize>,
    changed_fields: u32,
    incomparable_fields: u32,
    _reservation: Reservation,
}

impl StoredObject {
    fn summary(&self) -> ObjectSummary<'_> {
        ObjectSummary {
            difference: self.difference.as_ref(),
            field_count: self.fields.len() as u32,
            changed_fields: self.changed_fields,
            incomparable_fields: self.incomparable_fields,
        }
    }

    fn identity(&self) -> &RelationIdentity {
        match &self.difference {
            SummaryDifference::Equal { source, .. }
            | SummaryDifference::Changed { source, .. }
            | SummaryDifference::SourceOnly { source } => source,
            SummaryDifference::TargetOnly { target } => target,
            SummaryDifference::NotComparable { observed, .. } => match observed {
                ObservedSides::Both { source, .. } | ObservedSides::Source { source } => source,
                ObservedSides::Target { target } => target,
            },
        }
    }

    fn observes(&self, identity: &RelationIdentity) -> bool {
        match &self.difference {
            SummaryDifference::Equal { source, target }
            | SummaryDifference::Changed { source, target } => {
                source == identity || target == identity
            }
            SummaryDifference::SourceOnly { source } => source == identity,
            SummaryDifference::TargetOnly { target } => target == identity,
            SummaryDifference::NotComparable { observed, .. } => match observed {
                ObservedSides::Both { source, target } => source == identity || target == identity,
                ObservedSides::Source { source } => source == identity,
                ObservedSides::Target { target } => target == identity,
            },
        }
    }

    fn observed_identity(&self, side: Side) -> Option<&RelationIdentity> {
        match (&self.difference, side) {
            (
                SummaryDifference::Equal { source, .. }
                | SummaryDifference::Changed { source, .. }
                | SummaryDifference::SourceOnly { source },
                Side::Source,
            ) => Some(source),
            (
                SummaryDifference::Equal { target, .. }
                | SummaryDifference::Changed { target, .. }
                | SummaryDifference::TargetOnly { target },
                Side::Target,
            ) => Some(target),
            (SummaryDifference::NotComparable { observed, .. }, side) => match (observed, side) {
                (ObservedSides::Both { source, .. }, Side::Source)
                | (ObservedSides::Source { source }, Side::Source) => Some(source),
                (ObservedSides::Both { target, .. }, Side::Target)
                | (ObservedSides::Target { target }, Side::Target) => Some(target),
                _ => None,
            },
            _ => None,
        }
    }

    fn eligibility(&self, identity: &RelationIdentity, side: Side) -> Option<&Eligibility> {
        if self.observed_identity(side) != Some(identity) {
            return None;
        }
        match side {
            Side::Source => self.source_eligibility.as_ref(),
            Side::Target => self.target_eligibility.as_ref(),
        }
    }
}

/// Owns each result value once after input captures have been dropped. Definition
/// text can leave only through value chunks, never object or field summaries.
/// The future job owner must run `compare` on a worker and retain its admission
/// reservation until that worker joins, including after cancellation.
pub struct Comparison {
    metadata: ComparisonMetadata,
    kind: DifferenceKind,
    source_excluded_counts: Vec<ExcludedCount>,
    target_excluded_counts: Vec<ExcludedCount>,
    objects: Vec<StoredObject>,
    fields: Vec<StoredField>,
    values: Values,
    created: Instant,
    _reservation: Reservation,
}

impl Comparison {
    pub fn metadata(&self) -> &ComparisonMetadata {
        &self.metadata
    }
    /// Equality is always restricted to metadata.coverage.scope. Any excluded
    /// counterpart or incomparable field prevents an overall Equal result.
    pub fn kind(&self) -> DifferenceKind {
        self.kind
    }
    pub fn object_count(&self) -> usize {
        self.objects.len()
    }
    pub fn excluded_counts(&self, side: Side) -> &[ExcludedCount] {
        match side {
            Side::Source => &self.source_excluded_counts,
            Side::Target => &self.target_excluded_counts,
        }
    }

    /// Returns the captured eligibility for one exact observed relation side.
    /// This native detail remains bound to the immutable result and its TTL.
    pub(crate) fn relation_eligibility(
        &self,
        identity: &ResultIdentity,
        relation: &RelationIdentity,
        side: Side,
        now: Instant,
    ) -> Result<&Eligibility, CompareError> {
        self.validate_read(identity, now)?;
        let index = self
            .objects
            .binary_search_by(|row| row.identity().name.cmp(&relation.name))
            .map_err(|_| CompareError::Unavailable)?;
        self.objects[index]
            .eligibility(relation, side)
            .ok_or(CompareError::Unavailable)
    }

    pub(crate) fn validate_read(
        &self,
        identity: &ResultIdentity,
        now: Instant,
    ) -> Result<(), CompareError> {
        if *identity != self.metadata.identity
            || now.saturating_duration_since(self.created) >= RESULT_TTL
        {
            Err(CompareError::Unavailable)
        } else {
            Ok(())
        }
    }

    pub fn object_page(
        &self,
        identity: &ResultIdentity,
        response_id: &str,
        offset: u32,
        now: Instant,
    ) -> Result<EncodedPage, CompareError> {
        self.validate_read(identity, now)?;
        let (range, next) = page_range(self.objects.len(), offset)?;
        let budget = self.values.result_budget();
        let _scratch = budget.scratch(range.len() * size_of::<ObjectSummary<'_>>())?;
        let summaries: Vec<_> = self.objects[range]
            .iter()
            .map(StoredObject::summary)
            .collect();
        pages::object_window(identity, response_id, &summaries, offset, next, &budget)
    }

    pub fn field_page(
        &self,
        identity: &ResultIdentity,
        object: &RelationIdentity,
        response_id: &str,
        offset: u32,
        now: Instant,
    ) -> Result<EncodedPage, CompareError> {
        self.validate_read(identity, now)?;
        let index = self
            .objects
            .binary_search_by(|row| row.identity().name.cmp(&object.name))
            .map_err(|_| CompareError::Unavailable)?;
        let row = &self.objects[index];
        if !row.observes(object) {
            return Err(CompareError::Unavailable);
        }
        let (range, next) = page_range(row.fields.len(), offset)?;
        let budget = self.values.result_budget();
        let _scratch = budget.scratch(range.len() * size_of::<FieldSummary<'_>>())?;
        let summaries: Vec<_> = self.fields
            [row.fields.start + range.start..row.fields.start + range.end]
            .iter()
            .map(|field| FieldSummary {
                path: &field.path,
                difference: field.difference,
            })
            .collect();
        pages::field_window(identity, response_id, &summaries, offset, next, &budget)
    }

    pub fn value_chunk(
        &self,
        request: &ValueRequest,
        response_id: &str,
        now: Instant,
    ) -> Result<EncodedPage, CompareError> {
        self.validate_read(&request.identity, now)?;
        self.values.chunk(request, response_id, now)
    }
}

fn page_range(len: usize, offset: u32) -> Result<(Range<usize>, Option<u32>), CompareError> {
    let start = offset as usize;
    if start > len {
        return Err(CompareError::InvalidRequest);
    }
    let end = (start + PAGE_ITEMS).min(len);
    Ok((start..end, (end < len).then_some(end as u32)))
}

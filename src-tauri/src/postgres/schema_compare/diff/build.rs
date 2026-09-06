use super::*;
use crate::postgres::schema_compare::{
    capture::{CaptureControl, CapturedEndpoint, CapturedField},
    normalize::{self, Fact, FieldDifference, MatchKind, ObjectMatch},
};
use std::cmp::Ordering;

/// Consumes two successful captures from one shared result budget. Same stored
/// connection endpoints must have been read in the same actual transaction.
/// Cancellation/deadline checks occur between objects and fields, including
/// both passes and the bounded sorts. No partial result escapes on failure.
pub fn compare(
    identity: ResultIdentity,
    mut source: CapturedEndpoint,
    mut target: CapturedEndpoint,
    control: &CaptureControl,
    created: Instant,
) -> Result<Comparison, CompareError> {
    control.check()?;
    identity.validate()?;
    let budget = source.result_budget().clone();
    if !budget.same_result_scope(target.result_budget()) {
        return Err(CompareError::InvalidRequest);
    }
    let consistency = validate_captures(&source, &target)?;
    let matches =
        normalize::match_inventory_checked(&source.inventory, &target.inventory, &budget, || {
            control.check()
        })?;
    let left = IndexedFields::new(&source, &budget, control)?;
    let right = IndexedFields::new(&target, &budget, control)?;
    let versions = (
        source.metadata.server_version_num,
        target.metadata.server_version_num,
    );

    // Count the exact union before allocating the retained field container.
    // This avoids reserving two complete outputs for equal schemas.
    let mut field_count = 0;
    for object in matches.objects() {
        control.check()?;
        if object.kind != MatchKind::NotComparable {
            visit_fields(
                left.for_object(object.source)?,
                right.for_object(object.target)?,
                versions,
                control,
                |_| {
                    field_count += 1;
                    Ok(())
                },
            )?;
        }
    }
    let metadata_bytes = metadata_bytes(&source)
        + metadata_bytes(&target)
        + identity.job_id.capacity()
        + identity.result_id.capacity()
        + SCOPE.len()
        + (source.coverage.excluded_categories.len() + target.coverage.excluded_categories.len())
            * size_of::<ExcludedCategory>()
        + source.excluded_counts.capacity() * size_of::<ExcludedCount>()
        + target.excluded_counts.capacity() * size_of::<ExcludedCount>();
    let base = budget.reserve(
        size_of::<Comparison>()
            + metadata_bytes
            + field_count * size_of::<StoredField>()
            + matches.objects().len() * size_of::<StoredObject>(),
    )?;
    let mut values = Values::new(&identity, &budget, created)?;
    let mut fields = Vec::with_capacity(field_count);
    let mut objects = Vec::with_capacity(matches.objects().len());
    let mut kind = DifferenceKind::Equal;
    let mut incomparable_fields = 0;
    for object in matches.objects() {
        control.check()?;
        let start = fields.len();
        let mut changed = 0;
        let mut incomparable = 0;
        let mut reason = None;
        if object.kind != MatchKind::NotComparable {
            visit_fields(
                left.for_object(object.source)?,
                right.for_object(object.target)?,
                versions,
                control,
                |field| {
                    let field_reason = incomparable_reason(field.source)
                        .or_else(|| incomparable_reason(field.target));
                    match field.kind {
                        DifferenceKind::SourceOnly
                        | DifferenceKind::TargetOnly
                        | DifferenceKind::Changed => changed += 1,
                        DifferenceKind::NotComparable => {
                            incomparable += 1;
                            reason = reason.or(field_reason);
                        }
                        DifferenceKind::Equal => (),
                    }
                    let reservation = budget.reserve(path_bytes(field.path))?;
                    let source = field
                        .source
                        .map(|fact| values.insert_fact(Side::Source, fact))
                        .transpose()?;
                    let target = field
                        .target
                        .map(|fact| values.insert_fact(Side::Target, fact))
                        .transpose()?;
                    let difference = summary_difference(field.kind, source, target, field_reason)?;
                    fields.push(StoredField {
                        path: field.path.clone(),
                        difference,
                        _reservation: reservation,
                    });
                    Ok(())
                },
            )?;
        }
        let object_kind = match object.kind {
            MatchKind::SourceOnly => DifferenceKind::SourceOnly,
            MatchKind::TargetOnly => DifferenceKind::TargetOnly,
            MatchKind::NotComparable => DifferenceKind::NotComparable,
            MatchKind::Matched if changed > 0 => DifferenceKind::Changed,
            MatchKind::Matched if incomparable > 0 => DifferenceKind::NotComparable,
            MatchKind::Matched => DifferenceKind::Equal,
        };
        kind = aggregate_kind(kind, object_kind);
        incomparable_fields += incomparable;
        let reservation = budget.reserve(
            object.source.map_or(0, |s| s.name.len()) + object.target.map_or(0, |t| t.name.len()),
        )?;
        objects.push(StoredObject {
            difference: object_difference(object, object_kind, object.reason.or(reason))?,
            source_eligibility: captured_eligibility(&source, object.source)?,
            target_eligibility: captured_eligibility(&target, object.target)?,
            fields: start..fields.len(),
            changed_fields: changed,
            incomparable_fields: incomparable,
            _reservation: reservation,
        });
    }
    control.check()?;
    let mut excluded_categories = Vec::with_capacity(
        source.coverage.excluded_categories.len() + target.coverage.excluded_categories.len(),
    );
    excluded_categories.extend_from_slice(&source.coverage.excluded_categories);
    excluded_categories.extend_from_slice(&target.coverage.excluded_categories);
    excluded_categories.sort_unstable();
    excluded_categories.dedup();
    let coverage = Coverage {
        scope: SCOPE.into(),
        normalization_version: NORMALIZATION_VERSION,
        excluded_relations: source
            .inventory
            .iter()
            .chain(&target.inventory)
            .filter(|r| r.eligibility != Eligibility::Eligible)
            .count() as u32,
        incomparable_fields,
        excluded_categories,
    };
    // End all loans before moving metadata and releasing capture containers.
    drop(left);
    drop(right);
    drop(matches);
    Ok(Comparison {
        metadata: ComparisonMetadata {
            identity,
            source: source.metadata.clone(),
            target: target.metadata.clone(),
            consistency,
            coverage,
        },
        kind,
        source_excluded_counts: std::mem::take(&mut source.excluded_counts),
        target_excluded_counts: std::mem::take(&mut target.excluded_counts),
        objects,
        fields,
        values,
        created,
        _reservation: base,
    })
}

fn captured_eligibility(
    capture: &CapturedEndpoint,
    identity: Option<&RelationIdentity>,
) -> Result<Option<Eligibility>, CompareError> {
    identity
        .map(|identity| {
            capture
                .inventory
                .iter()
                .find(|entry| entry.identity == *identity)
                .map(|entry| entry.eligibility.clone())
                .ok_or(CompareError::Unavailable)
        })
        .transpose()
}

fn validate_captures(
    source: &CapturedEndpoint,
    target: &CapturedEndpoint,
) -> Result<SnapshotConsistency, CompareError> {
    for (side, capture) in [(Side::Source, source), (Side::Target, target)] {
        if capture.metadata.server_version_num / 10_000 != 16 {
            return Err(CompareError::UnsupportedVersion {
                side,
                version: capture.metadata.server_version.clone(),
            });
        }
        if capture.coverage.scope != SCOPE
            || capture.coverage.normalization_version != NORMALIZATION_VERSION
            || capture.coverage.excluded_categories.len() > 13
        {
            return Err(CompareError::InvalidRequest);
        }
    }
    let same_connection =
        source.metadata.endpoint.connection_id == target.metadata.endpoint.connection_id;
    if same_connection != source.shares_snapshot(target) {
        return Err(CompareError::InvalidRequest);
    }
    Ok(if same_connection {
        SnapshotConsistency::SharedTransaction
    } else {
        SnapshotConsistency::IndependentTransactions
    })
}

struct IndexedFields<'a> {
    capture: &'a CapturedEndpoint,
    fields: Vec<&'a CapturedField>,
    _reservation: Reservation,
}

impl<'a> IndexedFields<'a> {
    fn new(
        capture: &'a CapturedEndpoint,
        budget: &Budget,
        control: &CaptureControl,
    ) -> Result<Self, CompareError> {
        if capture.fields().len() > MAX_VALUES {
            return Err(CompareError::LimitExceeded {
                limit: Limit::ChildFacts,
            });
        }
        let reservation = budget.scratch(
            capture.fields().len() * size_of::<&CapturedField>()
                + capture.inventory.len() * size_of::<u32>(),
        )?;
        let mut oids = Vec::with_capacity(capture.inventory.len());
        for entry in &capture.inventory {
            control.check()?;
            if entry.eligibility == Eligibility::Eligible {
                oids.push(
                    capture
                        .local_oid(&entry.identity)
                        .ok_or(CompareError::Unavailable)?,
                );
            }
        }
        oids.sort_unstable();
        if oids.windows(2).any(|p| p[0] == p[1]) {
            return Err(CompareError::Unavailable);
        }
        let mut fields = Vec::with_capacity(capture.fields().len());
        for field in capture.fields() {
            control.check()?;
            if oids.binary_search(&field.table_oid).is_err() {
                return Err(CompareError::Unavailable);
            }
            fields.push(field);
        }
        // The 50,000-record ceiling bounds each allocation-free sort. Each
        // comparison touches only a local OID and bounded identifier path.
        fields.sort_unstable_by(|a, b| a.table_oid.cmp(&b.table_oid).then(a.path.cmp(&b.path)));
        control.check()?;
        if fields
            .windows(2)
            .any(|p| p[0].table_oid == p[1].table_oid && p[0].path == p[1].path)
        {
            return Err(CompareError::Unavailable);
        }
        Ok(Self {
            capture,
            fields,
            _reservation: reservation,
        })
    }

    fn for_object(
        &self,
        identity: Option<&RelationIdentity>,
    ) -> Result<&[&'a CapturedField], CompareError> {
        let Some(identity) = identity else {
            return Ok(&[]);
        };
        let oid = self
            .capture
            .local_oid(identity)
            .ok_or(CompareError::Unavailable)?;
        let start = self.fields.partition_point(|f| f.table_oid < oid);
        let end = self.fields.partition_point(|f| f.table_oid <= oid);
        if start == end {
            return Err(CompareError::Unavailable);
        }
        Ok(&self.fields[start..end])
    }
}

pub(super) fn visit_fields(
    source: &[&CapturedField],
    target: &[&CapturedField],
    versions: (u32, u32),
    control: &CaptureControl,
    mut visit: impl FnMut(FieldDifference<'_>) -> Result<(), CompareError>,
) -> Result<(), CompareError> {
    let (mut left, mut right) = (0, 0);
    while left < source.len() || right < target.len() {
        control.check()?;
        let (s, t) = (source.get(left).copied(), target.get(right).copied());
        let ordering = match (s, t) {
            (Some(s), Some(t)) => s.path.cmp(&t.path),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => break,
        };
        let (s, t) = match ordering {
            Ordering::Less => {
                left += 1;
                (s, None)
            }
            Ordering::Greater => {
                right += 1;
                (None, t)
            }
            Ordering::Equal => {
                left += 1;
                right += 1;
                (s, t)
            }
        };
        let path = &s.or(t).expect("one observed field").path;
        let source = s.map(|f| f.value.fact(versions.0, versions.1));
        let target = t.map(|f| f.value.fact(versions.1, versions.0));
        visit(FieldDifference {
            path,
            source,
            target,
            kind: normalize::difference_kind(source, target),
        })?;
    }
    Ok(())
}

fn summary_difference<T>(
    kind: DifferenceKind,
    source: Option<T>,
    target: Option<T>,
    reason: Option<IncomparableReason>,
) -> Result<SummaryDifference<T>, CompareError> {
    Ok(match (kind, source, target) {
        (DifferenceKind::Equal, Some(source), Some(target)) => {
            SummaryDifference::Equal { source, target }
        }
        (DifferenceKind::Changed, Some(source), Some(target)) => {
            SummaryDifference::Changed { source, target }
        }
        (DifferenceKind::SourceOnly, Some(source), None) => {
            SummaryDifference::SourceOnly { source }
        }
        (DifferenceKind::TargetOnly, None, Some(target)) => {
            SummaryDifference::TargetOnly { target }
        }
        (DifferenceKind::NotComparable, source, target) => SummaryDifference::NotComparable {
            reason: reason.ok_or(CompareError::Unavailable)?,
            observed: match (source, target) {
                (Some(source), Some(target)) => ObservedSides::Both { source, target },
                (Some(source), None) => ObservedSides::Source { source },
                (None, Some(target)) => ObservedSides::Target { target },
                _ => return Err(CompareError::Unavailable),
            },
        },
        _ => return Err(CompareError::Unavailable),
    })
}

fn object_difference(
    object: &ObjectMatch<'_>,
    kind: DifferenceKind,
    reason: Option<IncomparableReason>,
) -> Result<SummaryDifference<RelationIdentity>, CompareError> {
    summary_difference(kind, object.source.cloned(), object.target.cloned(), reason)
}

fn incomparable_reason(fact: Option<Fact<'_>>) -> Option<IncomparableReason> {
    match fact {
        Some(Fact::NotComparable { reason, .. }) => Some(reason),
        _ => None,
    }
}

fn aggregate_kind(current: DifferenceKind, next: DifferenceKind) -> DifferenceKind {
    match (current, next) {
        (DifferenceKind::Changed, _)
        | (_, DifferenceKind::Changed | DifferenceKind::SourceOnly | DifferenceKind::TargetOnly) => {
            DifferenceKind::Changed
        }
        (DifferenceKind::NotComparable, _) | (_, DifferenceKind::NotComparable) => {
            DifferenceKind::NotComparable
        }
        _ => DifferenceKind::Equal,
    }
}

fn path_bytes(path: &FieldPath) -> usize {
    match path {
        FieldPath::Table { .. } => 0,
        FieldPath::Column { name, .. } | FieldPath::Constraint { name, .. } => name.len(),
        FieldPath::Index { name, owner, .. } | FieldPath::IndexKey { name, owner, .. } => {
            name.len() + owner.as_ref().map_or(0, String::len)
        }
    }
}

fn metadata_bytes(capture: &CapturedEndpoint) -> usize {
    let m = &capture.metadata;
    m.endpoint.connection_id.len()
        + m.endpoint.schema.len()
        + m.server_version.len()
        + m.captured_at.len()
}

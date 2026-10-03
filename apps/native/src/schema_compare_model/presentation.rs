use super::*;
use reader::{field_sides, object_sides};
pub fn phase_label(state: &StatusState) -> &'static str {
    match state {
        StatusState::Resolving => "Resolving endpoints",
        StatusState::ReadingSource => "Reading source",
        StatusState::ReadingTarget => "Reading target",
        StatusState::ReadingBoth => "Reading both schemas in one transaction",
        StatusState::Comparing => "Comparing definitions",
        StatusState::Completed { .. } => "Completed",
        StatusState::Cancelling => "Cancelling; waiting for owned work",
        StatusState::Cancelled => "Cancelled",
        StatusState::Failed { .. } => "Failed",
    }
}
pub fn difference_label<T>(difference: &SummaryDifference<T>) -> &'static str {
    match difference {
        SummaryDifference::Equal { .. } => "Equal within scope",
        SummaryDifference::Changed { .. } => "Changed",
        SummaryDifference::SourceOnly { .. } => "Source only",
        SummaryDifference::TargetOnly { .. } => "Target only",
        SummaryDifference::NotComparable { .. } => "Not comparable",
    }
}
pub fn object_identity(item: &ObjectSummary) -> &RelationIdentity {
    let (source, target) = object_sides(item);
    source
        .or(target)
        .expect("typed difference always carries an observed side")
}
fn words(value: String) -> String {
    let mut output = String::with_capacity(value.len() + 8);
    for (index, c) in value.chars().enumerate() {
        if index > 0 && c.is_ascii_uppercase() {
            output.push(' ');
        }
        output.extend(c.to_lowercase());
    }
    output
}
pub fn field_label(path: &FieldPath) -> String {
    match path {
        FieldPath::Table { field } => format!("Table / {}", words(format!("{field:?}"))),
        FieldPath::Column { name, field } => {
            format!("Column / {name} / {}", words(format!("{field:?}")))
        }
        FieldPath::Constraint { name, field } => {
            format!("Constraint / {name} / {}", words(format!("{field:?}")))
        }
        FieldPath::Index { name, owner, field } => format!(
            "Index / {name}{} / {}",
            owner
                .as_ref()
                .map_or_else(String::new, |owner| format!(" (constraint {owner})")),
            words(format!("{field:?}"))
        ),
        FieldPath::IndexKey {
            name,
            owner,
            position,
            field,
        } => format!(
            "Index key / {name}{} #{} / {}",
            owner
                .as_ref()
                .map_or_else(String::new, |owner| format!(" (constraint {owner})")),
            u32::from(*position) + 1,
            words(format!("{field:?}"))
        ),
    }
}
pub fn incomparable_label(reason: IncomparableReason) -> &'static str {
    match reason {
        IncomparableReason::ExpressionOutsideSubset => {
            "Expression outside the supported scalar grammar. Identical raw text does not establish equality."
        }
        IncomparableReason::RenderingVersionDifference => {
            "Different PostgreSQL 16 minor versions; rendered expressions are not comparable."
        }
        IncomparableReason::ExternalDependency => {
            "Depends on an object whose definition is outside this comparison."
        }
        IncomparableReason::UnknownAccessMethod => "Index access method is not recognized.",
        IncomparableReason::ExcludedCounterpart => {
            "The counterpart is excluded from scope; this is not directional absence."
        }
        IncomparableReason::ExcludedObject => {
            "Object excluded from scope; no equality or absence is inferred."
        }
    }
}
pub fn coverage_text(reply: &CompareReply) -> Option<String> {
    let CompareReply::Metadata {
        metadata,
        kind,
        object_count,
        source_excluded_counts,
        target_excluded_counts,
    } = reply
    else {
        return None;
    };
    let coverage = &metadata.coverage;
    let mut text = format!(
        "{} · {object_count} objects\nPostgreSQL 16 ordinary tables: columns, constraints, indexes, table persistence and comments. Type and collation references compare names, not their definitions.\nScope: {} · normalization {}\nIncomparable fields: {} · excluded relations: {}\nSource: {}.{} · {} · {}\nTarget: {}.{} · {} · {}\n{}\nNot compared: {}",
        match kind {
            DifferenceKind::Equal => "Equal within scope",
            DifferenceKind::Changed => "Changed",
            DifferenceKind::SourceOnly => "Source only",
            DifferenceKind::TargetOnly => "Target only",
            DifferenceKind::NotComparable => "Not comparable",
        },
        coverage.scope,
        coverage.normalization_version,
        coverage.incomparable_fields,
        coverage.excluded_relations,
        metadata.source.endpoint.connection_id,
        metadata.source.endpoint.schema,
        metadata.source.server_version,
        metadata.source.captured_at,
        metadata.target.endpoint.connection_id,
        metadata.target.endpoint.schema,
        metadata.target.server_version,
        metadata.target.captured_at,
        match metadata.consistency {
            SnapshotConsistency::SharedTransaction =>
                "Both schemas share one transaction on the same stored connection.",
            SnapshotConsistency::IndependentTransactions =>
                "Independent capture times; this is not one cross-database snapshot.",
        },
        coverage
            .excluded_categories
            .iter()
            .map(|category| words(format!("{category:?}")))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (label, counts) in [
        ("Source", source_excluded_counts),
        ("Target", target_excluded_counts),
    ] {
        for count in counts {
            text.push_str(&format!(
                "\n{label} {}: {}",
                words(format!("{:?}", count.category)),
                if count.complete {
                    count.count.to_string()
                } else if count.count > 0 {
                    format!("At least {}", count.count)
                } else {
                    "Not compared".into()
                }
            ));
        }
    }
    if *object_count == 0 {
        text.push_str("\nNo ordinary table objects in this result. Review exclusions; this does not establish complete schema equality.");
    }
    if metadata.source.server_version_num != metadata.target.server_version_num {
        text.push_str("\nPG16 minor versions differ. Structured facts remain comparable; rendered expressions do not establish equality or changes.");
    }
    Some(text)
}
pub enum ValueState<'a> {
    Absent,
    Excluded(Option<&'a Exclusion>),
    Null,
    Empty,
    Unread,
    Text {
        value: &'a str,
        offset: u32,
        next: u32,
        total: u32,
        complete: bool,
    },
}
pub fn value_state(state: &ReaderState, side: Side) -> ValueState<'_> {
    if let Some(Eligibility::Excluded { reason }) = state.eligibility(side) {
        return ValueState::Excluded(Some(reason));
    }
    if let Some(object) = state.selected_object_summary()
        && matches!(
            object.difference,
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExcludedObject
                    | IncomparableReason::ExcludedCounterpart,
                ..
            }
        )
    {
        let (source, target) = object_sides(object);
        let observed = match side {
            Side::Source => source,
            Side::Target => target,
        };
        if observed.is_some() && state.eligibility(side).is_none() {
            return if observed.is_some_and(|identity| identity.kind != RelationKind::Table) {
                ValueState::Excluded(None)
            } else {
                ValueState::Unread
            };
        }
    }
    let Some(field) = state.selected_field_summary() else {
        return ValueState::Unread;
    };
    let (source, target) = field_sides(field);
    let value = match side {
        Side::Source => source,
        Side::Target => target,
    };
    let Some(value) = value else {
        return ValueState::Absent;
    };
    if value.value_kind == ValueKind::Null {
        return ValueState::Null;
    }
    if value.raw_bytes == 0 {
        return ValueState::Empty;
    }
    match state.value(side) {
        Some(CompareReply::Value {
            text,
            offset,
            next_offset,
            complete,
            ..
        }) => ValueState::Text {
            value: text,
            offset: *offset,
            next: *next_offset,
            total: value.raw_bytes,
            complete: *complete,
        },
        _ => ValueState::Unread,
    }
}

pub fn failure_text(error: &CompareError) -> String {
    match error {
        CompareError::Busy => "Another comparison is using an endpoint, or both comparison slots are active. Wait for those jobs to finish.".into(),
        CompareError::LimitExceeded { limit } => format!("The {} limit was exceeded. No complete result was produced.", match limit { Limit::Inventory => "inventory", Limit::Tables => "table count", Limit::ChildFacts => "column, constraint and index", Limit::FieldBytes => "field size", Limit::EndpointBytes => "endpoint size", Limit::ResultBytes => "result size", Limit::PageBytes => "page size", Limit::PageItems => "page item", Limit::Allocation => "memory" }),
        CompareError::UnsupportedVersion { side, version } => format!("{} is running PostgreSQL {version}. Schema comparison supports PostgreSQL 16 endpoints only.", side_name(*side)),
        CompareError::UnsupportedEngine { side } => format!("{} is not a PostgreSQL connection.", side_name(*side)),
        CompareError::Unavailable => "This comparison is unavailable. It may have expired, been dismissed or been invalidated by a connection change.".into(),
        CompareError::InvalidRequest => "The request was rejected as invalid. Check both connections and exact schema names.".into(),
        CompareError::CaptureChanged => "Definitions changed, or a table stayed locked, while being read. Run a new comparison to capture the current state.".into(),
        CompareError::Cancelled => "Comparison cancelled. No result was produced.".into(),
        CompareError::DeadlineExceeded => "The comparison did not finish within its time limit. No complete result was produced.".into(),
    }
}
pub fn job_failure_text(error: &CompareError) -> String {
    if matches!(error, CompareError::Unavailable) {
        return "An endpoint could not be read. The schema may not exist, catalog privileges may be missing, or the connection may have been lost or invalidated. No result was produced.".into();
    }
    let mut text = failure_text(error);
    if !text.ends_with("result was produced.") {
        text.push_str(" No result was produced.");
    }
    text
}
fn side_name(side: Side) -> &'static str {
    match side {
        Side::Source => "Source",
        Side::Target => "Target",
    }
}
pub fn value_kind_label(kind: ValueKind) -> &'static str {
    match kind {
        ValueKind::Null => "NULL",
        ValueKind::Text => "text",
        ValueKind::Boolean => "boolean",
        ValueKind::Integer => "integer",
        ValueKind::QualifiedName => "qualified name",
        ValueKind::OrderedNames => "ordered names",
        ValueKind::OperatorSignatures => "operator signatures",
    }
}
pub fn summary_text(reply: &CompareReply) -> Option<String> {
    let CompareReply::Metadata {
        metadata,
        kind,
        object_count,
        ..
    } = reply
    else {
        return None;
    };
    let label = match kind {
        DifferenceKind::Equal => "Equal within scope",
        DifferenceKind::Changed => "Changed",
        DifferenceKind::SourceOnly => "Source only",
        DifferenceKind::TargetOnly => "Target only",
        DifferenceKind::NotComparable => "Not comparable",
    };
    Some(format!(
        "{label} · {object_count} objects · {} fields not comparable · {} excluded relations",
        metadata.coverage.incomparable_fields, metadata.coverage.excluded_relations
    ))
}
fn exclusion_label(reason: &Exclusion) -> &'static str {
    match reason {
        Exclusion::Partitioned => "partitioned relation",
        Exclusion::Inherited => "inherited relation",
        Exclusion::Foreign => "foreign relation",
        Exclusion::ExtensionOwned => "extension-owned relation",
        Exclusion::OtherKind => "relation kind outside scope",
    }
}
fn side_exclusion(state: &ReaderState, side: Side) -> Option<String> {
    if let Some(Eligibility::Excluded { reason }) = state.eligibility(side) {
        return Some(format!("Excluded: {}", exclusion_label(reason)));
    }
    let object = state.selected_object_summary()?;
    if !matches!(
        object.difference,
        SummaryDifference::NotComparable {
            reason: IncomparableReason::ExcludedObject | IncomparableReason::ExcludedCounterpart,
            ..
        }
    ) {
        return None;
    }
    let (source, target) = object_sides(object);
    let identity = match side {
        Side::Source => source,
        Side::Target => target,
    }?;
    if state.eligibility(side).is_none() {
        return Some(if identity.kind == RelationKind::Table {
            "Eligibility not loaded".into()
        } else {
            "Excluded: relation kind outside scope".into()
        });
    }
    None
}
pub fn object_detail(state: &ReaderState) -> Option<String> {
    let item = state.selected_object_summary()?;
    let mut text = format!(
        "Selected object: {} · {}",
        object_identity(item).name,
        difference_label(&item.difference)
    );
    let excluded = matches!(
        item.difference,
        SummaryDifference::NotComparable {
            reason: IncomparableReason::ExcludedObject | IncomparableReason::ExcludedCounterpart,
            ..
        }
    );
    if let SummaryDifference::NotComparable { reason, .. } = item.difference {
        text.push_str(&format!("\n{}", incomparable_label(reason)));
    }
    if excluded {
        let (source, target) = object_sides(item);
        for (side, identity) in [(Side::Source, source), (Side::Target, target)] {
            let detail = if identity.is_none() {
                "Not observed".into()
            } else {
                match state.eligibility(side) {
                    Some(Eligibility::Eligible) => "Eligible ordinary table".into(),
                    Some(Eligibility::Excluded { reason }) => {
                        format!("Excluded: {}", exclusion_label(reason))
                    }
                    None => "Eligibility not loaded".into(),
                }
            };
            text.push_str(&format!("\n{}: {detail}", side_name(side)));
        }
    }
    if item.field_count == 0 {
        text.push_str(if excluded {
            "\nNo fields are compared for an excluded definition."
        } else {
            "\nNo comparable fields in scope for this object."
        });
    }
    Some(text)
}
pub fn field_side_label(state: &ReaderState, field: &FieldSummary, side: Side) -> String {
    let (source, target) = field_sides(field);
    let value = match side {
        Side::Source => source,
        Side::Target => target,
    };
    let Some(value) = value else {
        return side_exclusion(state, side).unwrap_or_else(|| "Absent".into());
    };
    if value.value_kind == ValueKind::Null {
        return "NULL (observed)".into();
    }
    if value.raw_bytes == 0 {
        return format!(
            "{} · Empty string (0 B)",
            value_kind_label(value.value_kind)
        );
    }
    let mut label = format!(
        "{} · {} B",
        value_kind_label(value.value_kind),
        value.raw_bytes
    );
    if state.selected_field() == Some(&field.path) {
        if let Some(CompareReply::Value {
            offset,
            next_offset,
            ..
        }) = state.value(side)
        {
            label.push_str(&format!(" · loaded bytes {offset}–{next_offset}"));
        } else {
            label.push_str(" · Not loaded");
        }
    } else {
        label.push_str(" · Select to inspect");
    }
    label
}

use super::{
    budget::*,
    protocol::*,
    values::{encode, validate_response_id, EncodedPage, ValueRef},
};
use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(untagged)]
pub enum ObservedSides<T> {
    Both { source: T, target: T },
    Source { source: T },
    Target { target: T },
}

#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SummaryDifference<T> {
    Equal {
        source: T,
        target: T,
    },
    Changed {
        source: T,
        target: T,
    },
    SourceOnly {
        source: T,
    },
    TargetOnly {
        target: T,
    },
    NotComparable {
        reason: IncomparableReason,
        #[serde(flatten)]
        observed: ObservedSides<T>,
    },
}

impl<T> SummaryDifference<T> {
    pub(crate) fn as_ref(&self) -> SummaryDifference<&T> {
        match self {
            Self::Equal { source, target } => SummaryDifference::Equal { source, target },
            Self::Changed { source, target } => SummaryDifference::Changed { source, target },
            Self::SourceOnly { source } => SummaryDifference::SourceOnly { source },
            Self::TargetOnly { target } => SummaryDifference::TargetOnly { target },
            Self::NotComparable { reason, observed } => SummaryDifference::NotComparable {
                reason: *reason,
                observed: match observed {
                    ObservedSides::Both { source, target } => {
                        ObservedSides::Both { source, target }
                    }
                    ObservedSides::Source { source } => ObservedSides::Source { source },
                    ObservedSides::Target { target } => ObservedSides::Target { target },
                },
            },
        }
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSummary<'a> {
    pub path: &'a FieldPath,
    #[serde(flatten)]
    pub difference: SummaryDifference<ValueRef>,
}

impl FieldSummary<'_> {
    fn validate(&self) -> Result<(), CompareError> {
        let valid = match self.difference {
            SummaryDifference::Equal { source, target }
            | SummaryDifference::Changed { source, target } => {
                source.side == Side::Source && target.side == Side::Target
            }
            SummaryDifference::SourceOnly { source } => source.side == Side::Source,
            SummaryDifference::TargetOnly { target } => target.side == Side::Target,
            SummaryDifference::NotComparable { observed, .. } => match observed {
                ObservedSides::Both { source, target } => {
                    source.side == Side::Source && target.side == Side::Target
                }
                ObservedSides::Source { source } => source.side == Side::Source,
                ObservedSides::Target { target } => target.side == Side::Target,
            },
        };
        valid.then_some(()).ok_or(CompareError::InvalidRequest)
    }
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectSummary<'a> {
    #[serde(flatten)]
    pub difference: SummaryDifference<&'a RelationIdentity>,
    pub field_count: u32,
    pub changed_fields: u32,
    pub incomparable_fields: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page<'a, T> {
    response_id: &'a str,
    identity: &'a ResultIdentity,
    offset: u32,
    next_offset: Option<u32>,
    items: &'a [T],
}

/// Caller lends summaries of one immutable result; page serialization does
/// not clone fields or definitions. End-of-list is distinct from bad offsets.
fn summaries<T: Serialize>(
    identity: &ResultIdentity,
    response_id: &str,
    all: &[T],
    offset: u32,
    budget: &Budget,
    validate: impl Fn(&T) -> Result<(), CompareError>,
) -> Result<EncodedPage, CompareError> {
    identity.validate()?;
    let start = offset as usize;
    if start > all.len() || all.len() > 2 * MAX_VALUES {
        return Err(CompareError::InvalidRequest);
    }
    let end = (start + PAGE_ITEMS).min(all.len());
    window(
        identity,
        response_id,
        &all[start..end],
        offset,
        (end < all.len()).then_some(end as u32),
        budget,
        validate,
    )
}

// A retained result projects only the requested window into borrowed summaries.
// Its full definition/summary arrays never enter the serializer's scratch.
fn window<T: Serialize>(
    identity: &ResultIdentity,
    response_id: &str,
    items: &[T],
    offset: u32,
    next_offset: Option<u32>,
    budget: &Budget,
    validate: impl Fn(&T) -> Result<(), CompareError>,
) -> Result<EncodedPage, CompareError> {
    identity.validate()?;
    validate_response_id(response_id)?;
    if items.len() > PAGE_ITEMS {
        return Err(CompareError::LimitExceeded {
            limit: Limit::PageItems,
        });
    }
    for item in items {
        validate(item)?;
    }
    let lease = budget.serializer()?;
    encode(
        &Page {
            response_id,
            identity,
            offset,
            next_offset,
            items,
        },
        response_id,
        lease,
    )
}

pub(crate) fn object_window(
    identity: &ResultIdentity,
    response_id: &str,
    items: &[ObjectSummary<'_>],
    offset: u32,
    next_offset: Option<u32>,
    budget: &Budget,
) -> Result<EncodedPage, CompareError> {
    window(
        identity,
        response_id,
        items,
        offset,
        next_offset,
        budget,
        |_| Ok(()),
    )
}

pub(crate) fn field_window(
    identity: &ResultIdentity,
    response_id: &str,
    items: &[FieldSummary<'_>],
    offset: u32,
    next_offset: Option<u32>,
    budget: &Budget,
) -> Result<EncodedPage, CompareError> {
    window(
        identity,
        response_id,
        items,
        offset,
        next_offset,
        budget,
        FieldSummary::validate,
    )
}

pub fn object_summaries(
    identity: &ResultIdentity,
    response_id: &str,
    all: &[ObjectSummary<'_>],
    offset: u32,
    budget: &Budget,
) -> Result<EncodedPage, CompareError> {
    summaries(identity, response_id, all, offset, budget, |_| Ok(()))
}

pub fn field_summaries(
    identity: &ResultIdentity,
    response_id: &str,
    all: &[FieldSummary<'_>],
    offset: u32,
    budget: &Budget,
) -> Result<EncodedPage, CompareError> {
    summaries(
        identity,
        response_id,
        all,
        offset,
        budget,
        FieldSummary::validate,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(side: Side, value_id: u32) -> ValueRef {
        ValueRef {
            side,
            value_id,
            raw_bytes: 1,
            value_kind: super::super::values::ValueKind::Text,
        }
    }

    #[test]
    fn pages_at_most_one_hundred_summaries_and_rejects_invalid_offsets() {
        let id = ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        };
        let relation = RelationIdentity {
            kind: RelationKind::Table,
            name: "orders".into(),
        };
        let data = vec![
            ObjectSummary {
                difference: SummaryDifference::Equal {
                    source: &relation,
                    target: &relation,
                },
                field_count: 1,
                changed_fields: 0,
                incomparable_fields: 0,
            };
            101
        ];
        let budget = Budget::default();
        let page = object_summaries(&id, "response", &data, 0, &budget).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(page.as_str()).unwrap();
        assert_eq!(decoded["items"].as_array().unwrap().len(), 100);
        assert_eq!(decoded["nextOffset"], 100);
        let last = object_summaries(&id, "response2", &data, 100, &budget).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(last.as_str()).unwrap();
        assert_eq!(decoded["items"].as_array().unwrap().len(), 1);
        assert!(decoded["nextOffset"].is_null());
        assert!(object_summaries(&id, "response3", &data, 102, &budget).is_err());
    }

    #[test]
    fn incomparable_summary_flattens_its_required_observed_sides() {
        let id = ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        };
        let source = RelationIdentity {
            kind: RelationKind::Table,
            name: "orders".into(),
        };
        let target = RelationIdentity {
            kind: RelationKind::View,
            name: "orders".into(),
        };
        let summaries = [ObjectSummary {
            difference: SummaryDifference::NotComparable {
                reason: IncomparableReason::ExcludedCounterpart,
                observed: ObservedSides::Both {
                    source: &source,
                    target: &target,
                },
            },
            field_count: 0,
            changed_fields: 0,
            incomparable_fields: 0,
        }];

        let page = object_summaries(&id, "response", &summaries, 0, &Budget::default()).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(page.as_str()).unwrap();
        let summary = &decoded["items"][0];
        assert_eq!(summary["kind"], "notComparable");
        assert_eq!(summary["reason"], "excludedCounterpart");
        assert_eq!(summary["source"]["kind"], "table");
        assert_eq!(summary["target"]["kind"], "view");
        assert!(summary.get("observed").is_none());
    }

    #[test]
    fn field_pages_reject_value_references_bound_to_the_wrong_side() {
        let id = ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        };
        let path = FieldPath::Table {
            field: TableField::Comment,
        };
        let source = value(Side::Source, 0);
        let target = value(Side::Target, 1);
        let invalid = [
            SummaryDifference::SourceOnly { source: target },
            SummaryDifference::Equal {
                source: target,
                target: source,
            },
            SummaryDifference::Changed {
                source: target,
                target: source,
            },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Both {
                    source: target,
                    target: source,
                },
            },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Source { source: target },
            },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Target { target: source },
            },
        ];
        let budget = Budget::default();

        for difference in invalid {
            let summaries = [FieldSummary {
                path: &path,
                difference,
            }];
            assert!(matches!(
                field_summaries(&id, "response", &summaries, 0, &budget),
                Err(CompareError::InvalidRequest)
            ));
            assert_eq!(budget.used(), 0);
        }
    }

    #[test]
    fn field_pages_accept_value_references_bound_to_their_named_sides() {
        let id = ResultIdentity {
            job_id: "job".into(),
            result_id: "result".into(),
        };
        let path = FieldPath::Table {
            field: TableField::Comment,
        };
        let source = value(Side::Source, 0);
        let target = value(Side::Target, 1);
        let summaries = [
            SummaryDifference::Equal { source, target },
            SummaryDifference::Changed { source, target },
            SummaryDifference::SourceOnly { source },
            SummaryDifference::TargetOnly { target },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Both { source, target },
            },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Source { source },
            },
            SummaryDifference::NotComparable {
                reason: IncomparableReason::ExpressionOutsideSubset,
                observed: ObservedSides::Target { target },
            },
        ]
        .map(|difference| FieldSummary {
            path: &path,
            difference,
        });

        let page = field_summaries(&id, "response", &summaries, 0, &Budget::default()).unwrap();
        let decoded: serde_json::Value = serde_json::from_str(page.as_str()).unwrap();
        assert_eq!(decoded["items"].as_array().unwrap().len(), summaries.len());
    }
}

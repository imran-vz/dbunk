use super::*;
use std::mem::size_of;
fn add(n: &mut usize, bytes: usize) -> Option<()> {
    *n = n.checked_add(bytes)?;
    (*n <= MAX_COMPARISON_PAGE_BYTES).then_some(())
}
fn text(n: &mut usize, value: &String, max: usize) -> Option<()> {
    if value.len() > max {
        return None;
    }
    add(n, value.capacity())
}
fn identifier(n: &mut usize, value: &String) -> Option<()> {
    if value.is_empty() || value.contains('\0') {
        return None;
    }
    text(n, value, 63)
}
fn identity(n: &mut usize, value: &ResultIdentity) -> Option<()> {
    value.validate().ok()?;
    text(n, &value.job_id, 128)?;
    text(n, &value.result_id, 128)
}
pub(super) fn request_bytes(value: &ResultRequest) -> Option<usize> {
    let mut n = size_of::<ResultRequest>();
    identity(&mut n, &value.identity)?;
    add(&mut n, types::endpoint_bytes(&value.source)?)?;
    add(&mut n, types::endpoint_bytes(&value.target)?)?;
    (n <= 2048).then_some(n)
}
fn object(n: &mut usize, value: &RelationIdentity) -> Option<()> {
    identifier(n, &value.name)
}
fn path(n: &mut usize, value: &FieldPath) -> Option<()> {
    match value {
        FieldPath::Table { .. } => Some(()),
        FieldPath::Column { name, .. } | FieldPath::Constraint { name, .. } => identifier(n, name),
        FieldPath::Index { name, owner, .. } | FieldPath::IndexKey { name, owner, .. } => {
            identifier(n, name)?;
            if let Some(owner) = owner {
                identifier(n, owner)?;
            }
            Some(())
        }
    }
}
fn value(v: &ValueRef) -> Option<()> {
    (v.raw_bytes <= 256 * 1024 && v.value_id < 100_000).then_some(())
}
pub(super) fn read_bytes(read: &ReadRequest) -> Option<usize> {
    let mut n = size_of::<ReadRequest>();
    match read {
        ReadRequest::Fields { object: o, .. } | ReadRequest::Eligibility { object: o, .. } => {
            object(&mut n, o)?
        }
        ReadRequest::Value { value: v, .. } => value(v)?,
        _ => {}
    };
    (n <= 1024).then_some(n)
}
fn sides<T>(
    diff: &SummaryDifference<T>,
    mut check: impl FnMut(Side, &T) -> Option<()>,
) -> Option<()> {
    match diff {
        SummaryDifference::Equal { source, target }
        | SummaryDifference::Changed { source, target } => {
            check(Side::Source, source)?;
            check(Side::Target, target)
        }
        SummaryDifference::SourceOnly { source } => check(Side::Source, source),
        SummaryDifference::TargetOnly { target } => check(Side::Target, target),
        SummaryDifference::NotComparable { observed, .. } => match observed {
            ObservedSides::Both { source, target } => {
                check(Side::Source, source)?;
                check(Side::Target, target)
            }
            ObservedSides::Source { source } => check(Side::Source, source),
            ObservedSides::Target { target } => check(Side::Target, target),
        },
    }
}
fn page(offset: u32, next: Option<u32>, len: usize) -> Option<()> {
    if len > 100 {
        return None;
    }
    match next {
        Some(next) if next == offset.checked_add(u32::try_from(len).ok()?)? && next > offset => {
            Some(())
        }
        None => Some(()),
        _ => None,
    }
}
fn counts(n: &mut usize, rows: &Vec<ExcludedCount>) -> Option<()> {
    if rows.len() > 13 {
        return None;
    }
    add(n, rows.capacity().checked_mul(size_of::<ExcludedCount>())?)?;
    for (i, row) in rows.iter().enumerate() {
        if rows[..i].iter().any(|x| x.category == row.category) {
            return None;
        }
    }
    Some(())
}
fn metadata(n: &mut usize, m: &ComparisonMetadata) -> Option<()> {
    identity(n, &m.identity)?;
    for c in [&m.source, &m.target] {
        add(n, types::endpoint_bytes(&c.endpoint)?)?;
        text(n, &c.server_version, 256)?;
        text(n, &c.captured_at, 128)?;
    }
    text(n, &m.coverage.scope, 128)?;
    if m.coverage.excluded_categories.len() > 13 {
        return None;
    }
    add(
        n,
        m.coverage
            .excluded_categories
            .capacity()
            .checked_mul(size_of::<ExcludedCategory>())?,
    )
}
/// Measures capacities and validates exact reply/request pairing without making
/// an intermediate JSON value or cloning definition strings.
pub(super) fn page_bytes(p: &SchemaComparisonPage) -> Option<usize> {
    let mut n = size_of::<SchemaComparisonPage>();
    if p.response_id.is_empty() {
        return None;
    }
    text(&mut n, &p.response_id, 128)?;
    add(&mut n, request_bytes(&p.request)?)?;
    add(&mut n, read_bytes(&p.read)?)?;
    match (&p.read, &p.reply) {
        (
            ReadRequest::Metadata,
            CompareReply::Metadata {
                metadata: m,
                source_excluded_counts,
                target_excluded_counts,
                ..
            },
        ) => {
            if m.identity != p.request.identity
                || m.source.endpoint != p.request.source
                || m.target.endpoint != p.request.target
            {
                return None;
            }
            metadata(&mut n, m)?;
            counts(&mut n, source_excluded_counts)?;
            counts(&mut n, target_excluded_counts)?;
        }
        (
            ReadRequest::Objects { offset: expected },
            CompareReply::Objects {
                offset,
                next_offset,
                items,
            },
        ) => {
            if expected != offset {
                return None;
            }
            page(*offset, *next_offset, items.len())?;
            add(
                &mut n,
                items.capacity().checked_mul(size_of::<ObjectSummary>())?,
            )?;
            for item in items {
                sides(&item.difference, |_, o| object(&mut n, o))?;
            }
        }
        (
            ReadRequest::Fields {
                object: expected,
                offset: expected_offset,
            },
            CompareReply::Fields {
                object: o,
                offset,
                next_offset,
                items,
            },
        ) => {
            if expected != o || expected_offset != offset {
                return None;
            }
            object(&mut n, o)?;
            page(*offset, *next_offset, items.len())?;
            add(
                &mut n,
                items.capacity().checked_mul(size_of::<FieldSummary>())?,
            )?;
            for item in items {
                path(&mut n, &item.path)?;
                sides(&item.difference, |side, v| {
                    if side != v.side {
                        return None;
                    }
                    value(v)
                })?;
            }
        }
        (
            ReadRequest::Eligibility {
                object: expected,
                side: expected_side,
            },
            CompareReply::Eligibility {
                object: o, side, ..
            },
        ) => {
            if expected != o || side != expected_side {
                return None;
            }
            object(&mut n, o)?;
        }
        (
            ReadRequest::Value {
                value: expected,
                offset: expected_offset,
            },
            CompareReply::Value {
                value: v,
                offset,
                text: chunk,
                next_offset,
                complete,
            },
        ) => {
            if expected != v
                || expected_offset != offset
                || *next_offset != offset.checked_add(u32::try_from(chunk.len()).ok()?)?
                || *next_offset > v.raw_bytes
                || *complete != (*next_offset == v.raw_bytes)
                || (!complete && chunk.is_empty())
            {
                return None;
            }
            value(v)?;
            text(&mut n, chunk, 64 * 1024)?;
        }
        _ => return None,
    }
    let mut encoded = Counter(0);
    serde_json::to_writer(&mut encoded, p).ok()?;
    Some(n)
}
struct Counter(usize);
impl std::io::Write for Counter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|n| *n <= MAX_COMPARISON_ENCODED_BYTES)
            .ok_or_else(|| std::io::Error::other("comparison page limit"))?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

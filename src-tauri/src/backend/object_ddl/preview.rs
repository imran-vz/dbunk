use super::*;
use crate::backend::schema_ddl::encoded_bytes;
use crate::postgres::object_ddl::{generate_object_ddl, StatementGroup};

/// FNV-1a 64 of the canonical operation JSON. A display fingerprint only.
pub(super) fn digest(operations: &[ObjectDdlOperation]) -> String {
    let bytes = serde_json::to_vec(operations).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("fnv64:{hash:016x}")
}

/// Regenerates statements from typed operations at every trust boundary. The
/// observed claims must answer each operation exactly; no SQL is accepted.
pub(super) fn render(
    target: &ObjectDdlDescription,
    operations: &[ObjectDdlOperation],
    timeout: Option<u32>,
    confirmation_required: bool,
) -> Result<ObjectDdlPreview, ObjectDdlError> {
    operations_heap_bytes(operations).ok_or(ObjectDdlError::InvalidRequest)?;
    target.checked_heap_bytes().ok_or(ObjectDdlError::Limit)?;
    if !target.answers(operations) {
        return Err(ObjectDdlError::TargetMismatch);
    }
    // Target+operations also form the receipt. Reserve escaped routing,
    // attempt/outcome fields and SQLSTATE/residue names before any review.
    let receipt_bound = target
        .encoded_bytes()
        .saturating_add(encoded_bytes(&operations))
        .saturating_add(256 * 6 + 1024);
    if receipt_bound > MAX_OBJECT_DDL_RECEIPT_BYTES {
        return Err(ObjectDdlError::Limit);
    }
    let typed = operations
        .iter()
        .map(ObjectDdlOperation::to_pg)
        .collect::<Vec<_>>();
    let plan = generate_object_ddl(&typed).map_err(|_| ObjectDdlError::InvalidRequest)?;
    if plan.statements.len() != operations.len() {
        return Err(ObjectDdlError::InvalidRequest);
    }
    let index = |value: usize| u16::try_from(value).map_err(|_| ObjectDdlError::Limit);
    let mut groups = Vec::with_capacity(plan.groups.len());
    for group in plan.groups {
        groups.push(match group {
            StatementGroup::Atomic { statement_indexes } => ObjectDdlGroup::Atomic {
                statements: statement_indexes
                    .into_iter()
                    .map(index)
                    .collect::<Result<_, _>>()?,
            },
            StatementGroup::Standalone { statement_index } => ObjectDdlGroup::Standalone {
                statement: index(statement_index)?,
            },
        });
    }
    let preview = ObjectDdlPreview {
        statements: plan
            .statements
            .into_iter()
            .map(|statement| ObjectDdlStatement {
                sql: statement.sql.into_boxed_str().into_string(),
                summary: statement.summary.into_boxed_str().into_string(),
                destructive: statement.destructive,
                transactional: statement.transactional,
            })
            .collect(),
        groups,
        operation_digest: digest(operations),
        confirmation_required,
        statement_timeout_ms: timeout,
        operation_timeout_ms: OBJECT_DDL_OPERATION_TIMEOUT_MS,
    };
    preview.checked_heap_bytes().ok_or(ObjectDdlError::Limit)?;
    Ok(preview)
}

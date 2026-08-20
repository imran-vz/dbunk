pub(crate) const CTID_COLUMN: &str = "ctid";
pub(crate) const TABLEOID_COLUMN: &str = "tableoid";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UniqueIndexCandidate {
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RelationIdentityKind {
    PrimaryKey,
    UniqueIndex,
    CtidFallback,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedIdentity {
    pub kind: RelationIdentityKind,
    pub columns: Vec<String>,
}

/// Resolve the authoritative PostgreSQL row identity shared by browse and
/// result mutation. Unique-index candidates are ordered here so callers cannot
/// accidentally drift from ADR-0022's smallest-index, then name tie-break.
pub(crate) fn resolve_identity(
    relkind: char,
    primary_key: &[String],
    unique_indexes: &[UniqueIndexCandidate],
) -> ResolvedIdentity {
    if !primary_key.is_empty() {
        return ResolvedIdentity {
            kind: RelationIdentityKind::PrimaryKey,
            columns: primary_key.to_vec(),
        };
    }

    let mut indexes = unique_indexes.to_vec();
    indexes.sort_by(|left, right| {
        left.columns
            .len()
            .cmp(&right.columns.len())
            .then_with(|| left.name.cmp(&right.name))
    });
    if let Some(index) = indexes.first() {
        return ResolvedIdentity {
            kind: RelationIdentityKind::UniqueIndex,
            columns: index.columns.clone(),
        };
    }

    match relkind {
        'p' => ResolvedIdentity {
            kind: RelationIdentityKind::CtidFallback,
            columns: vec![TABLEOID_COLUMN.to_string(), CTID_COLUMN.to_string()],
        },
        'r' => ResolvedIdentity {
            kind: RelationIdentityKind::CtidFallback,
            columns: vec![CTID_COLUMN.to_string()],
        },
        _ => ResolvedIdentity {
            kind: RelationIdentityKind::None,
            columns: Vec::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_shared_identity_precedence() {
        let indexes = vec![
            UniqueIndexCandidate {
                name: "z_two".into(),
                columns: vec!["a".into(), "b".into()],
            },
            UniqueIndexCandidate {
                name: "b_one".into(),
                columns: vec!["b".into()],
            },
            UniqueIndexCandidate {
                name: "a_one".into(),
                columns: vec!["a".into()],
            },
        ];

        assert_eq!(
            resolve_identity('r', &["id".into()], &indexes),
            ResolvedIdentity {
                kind: RelationIdentityKind::PrimaryKey,
                columns: vec!["id".into()],
            }
        );
        assert_eq!(
            resolve_identity('r', &[], &indexes),
            ResolvedIdentity {
                kind: RelationIdentityKind::UniqueIndex,
                columns: vec!["a".into()],
            }
        );
        assert_eq!(
            resolve_identity('p', &[], &[]),
            ResolvedIdentity {
                kind: RelationIdentityKind::CtidFallback,
                columns: vec!["tableoid".into(), "ctid".into()],
            }
        );
        assert_eq!(
            resolve_identity('v', &[], &[]),
            ResolvedIdentity {
                kind: RelationIdentityKind::None,
                columns: Vec::new(),
            }
        );
    }
}

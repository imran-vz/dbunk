use super::*;

fn address(object: i64) -> Address {
    Address::checked(1259, object, 0).unwrap()
}
fn candidate(object: i64, reported: bool) -> Candidate {
    Candidate {
        address: address(object),
        reported,
    }
}

#[test]
fn internal_nodes_are_walked_and_duplicate_paths_keep_reported_status() {
    let mut walk = Walk::new(address(1));
    walk.advance(
        vec![
            candidate(1, true),
            candidate(2, false),
            candidate(3, false),
            candidate(3, true),
        ],
        1,
        false,
    );
    assert_eq!(walk.frontier, [address(2), address(3)]);
    assert_eq!(walk.discovered, [(address(3), 1)]);
    walk.advance(
        vec![candidate(1, true), candidate(4, true), candidate(4, true)],
        2,
        false,
    );
    assert_eq!(walk.frontier, [address(4)]);
    assert_eq!(walk.discovered, [(address(3), 1), (address(4), 2)]);
    assert!(!walk.truncated);
}

#[test]
fn address_and_edge_caps_preserve_uncertainty_even_without_visible_rows() {
    let mut walk = Walk::new(address(1));
    walk.advance(
        (2..=(MAX_ADDRESSES_PER_DEPTH + 2) as i64)
            .map(|id| candidate(id, false))
            .collect(),
        1,
        false,
    );
    assert_eq!(walk.frontier.len(), MAX_ADDRESSES_PER_DEPTH);
    assert_eq!(walk.visited.len(), MAX_ADDRESSES_PER_DEPTH + 1);
    assert!(walk.discovered.is_empty());
    assert!(walk.truncated);
    let mut walk = Walk::new(address(1));
    walk.advance(vec![candidate(1, true)], 1, true);
    assert!(walk.frontier.is_empty());
    assert!(walk.truncated);
    let impact = Impact::new(walk.truncated).finish();
    assert!(impact.dependents.is_empty() && impact.truncated);
}

#[test]
fn depth_probe_distinguishes_a_complete_cycle_from_a_longer_chain() {
    let mut walk = Walk::new(address(1));
    for depth in 1..=MAX_DEPTH {
        walk.advance(vec![candidate(i64::from(depth) + 1, true)], depth, false);
    }
    assert_eq!(walk.discovered.last(), Some(&(address(9), 8)));
    walk.probe(&[candidate(1, true)], false);
    assert!(!walk.truncated);
    walk.probe(&[candidate(10, true)], false);
    assert!(walk.truncated);
    assert_eq!(walk.discovered.len(), 8);
}

#[test]
fn return_rules_normalize_but_custom_rules_and_column_addresses_remain_exact() {
    let rule = Address::checked(2618, 10, 0).unwrap();
    let view = Candidate::normalize(rule, "n", Some(20), 1259, false).unwrap();
    assert_eq!(view.address, address(20));
    assert!(view.reported);
    let custom = Candidate::normalize(rule, "n", None, 1259, false).unwrap();
    assert_eq!(custom.address, rule);
    let column = Address::checked(1259, 30, 2).unwrap();
    assert_eq!(
        Candidate::normalize(column, "n", None, 1259, false)
            .unwrap()
            .address,
        column
    );
    assert!(
        !Candidate::normalize(rule, "i", None, 1259, false)
            .unwrap()
            .reported
    );
    assert!(
        Candidate::normalize(address(40), "i", None, 1259, true)
            .unwrap()
            .reported
    );
    assert!(Candidate::normalize(rule, "e", None, 1259, false).is_err());
    assert!(Address::checked(0, 1, 0).is_err());
    assert!(Address::checked(1, 1, -1).is_err());
}

#[test]
fn output_cap_is_explicit_and_text_or_encoded_overflow_is_an_error() {
    let mut result = Impact::new(false);
    for id in 0..MAX_DROP_IMPACT_RESULTS + 1 {
        result
            .push("view", &format!("other_schema.\"view{id}\""), 2)
            .unwrap();
    }
    let result = result.finish();
    assert_eq!(result.dependents.len(), MAX_DROP_IMPACT_RESULTS);
    assert!(result.truncated);
    assert_eq!(result.dependents[0].identity, "other_schema.\"view0\"");
    let mut result = Impact::new(false);
    assert_eq!(
        result.push("table", &"x".repeat(MAX_TEXT_BYTES + 1), 1),
        Err(CatalogError::DropImpactLimit)
    );
    assert_eq!(
        result.push("table", "name", 0),
        Err(CatalogError::InvalidResponse)
    );
    // Text is individually in range, but JSON escaping must count in full.
    let escaped = "\u{1}".repeat(MAX_TEXT_BYTES);
    let mut refused = false;
    for _ in 0..MAX_DROP_IMPACT_RESULTS {
        if result.push("table", &escaped, 1) == Err(CatalogError::DropImpactLimit) {
            refused = true;
            break;
        }
    }
    assert!(refused);
}

#[test]
fn reference_validation_requires_exact_routine_signature_and_scoped_identity() {
    let mut reference = PgObjectRef {
        kind: PgObjectKind::Function,
        schema: Some("odd\"schema".into()),
        name: "name".into(),
        identity_args: Some("".into()),
    };
    assert!(description::validate(&reference).is_ok());
    reference.identity_args = None;
    assert_eq!(
        description::validate(&reference),
        Err(CatalogError::InvalidReference)
    );
    reference.kind = PgObjectKind::Table;
    reference.identity_args = Some("integer".into());
    assert_eq!(
        description::validate(&reference),
        Err(CatalogError::InvalidReference)
    );
    reference.kind = PgObjectKind::Schema;
    reference.identity_args = None;
    assert_eq!(
        description::validate(&reference),
        Err(CatalogError::InvalidReference)
    );
    reference.schema = None;
    assert!(description::validate(&reference).is_ok());
    reference.name = "name\0".into();
    assert_eq!(
        description::validate(&reference),
        Err(CatalogError::InvalidReference)
    );
}

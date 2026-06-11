//! Pure classification helpers for Schema Map relationship metadata.
//!
//! Everything here operates on rows already fetched by
//! `schema::fetch_schema_relationships` so the classification rules can
//! be unit-tested without a live PostgreSQL instance.

use std::collections::{HashMap, HashSet};

pub(crate) const CARDINALITY_ONE_TO_ONE: &str = "one-to-one";
pub(crate) const CARDINALITY_ONE_TO_MANY: &str = "one-to-many";

pub(crate) const RELATIONSHIP_TYPE_FOREIGN_KEY: &str = "foreign key";

/// Classify Relationship Cardinality from referencing-column
/// uniqueness, returning `(cardinality, reason)`.
///
/// `one-to-one` when the FK columns are constrained unique on the
/// referencing table, `one-to-many` otherwise. `unknown` is reserved
/// for engines that cannot provide uniqueness metadata and is never
/// produced here — PostgreSQL always knows.
pub(crate) fn classify_cardinality(fk_columns_unique: bool) -> (&'static str, &'static str) {
    if fk_columns_unique {
        (
            CARDINALITY_ONE_TO_ONE,
            "Referencing columns are constrained unique on the referencing table",
        )
    } else {
        (
            CARDINALITY_ONE_TO_MANY,
            "Referencing columns are not constrained unique on the referencing table",
        )
    }
}

/// Whether a unique constraint covers the FK columns: true when any
/// unique (non-partial, non-expression) index key set is a subset of
/// the FK column set — uniqueness of a subset implies uniqueness of
/// the superset.
pub(crate) fn unique_set_covers(fk_columns: &[String], unique_sets: &[Vec<String>]) -> bool {
    if fk_columns.is_empty() {
        return false;
    }
    let fk: HashSet<&str> = fk_columns.iter().map(String::as_str).collect();
    unique_sets
        .iter()
        .any(|set| !set.is_empty() && set.iter().all(|column| fk.contains(column.as_str())))
}

/// One outgoing FK as seen by junction detection.
pub(crate) struct OutgoingFk<'a> {
    pub constraint: &'a str,
    pub table: &'a str,
    pub columns: &'a [String],
}

/// Junction detection result: tables that act as many-to-many
/// associations, plus the FK constraints participating in those paths.
/// Participants are keyed by `(table, constraint)` — PostgreSQL
/// constraint names are only unique per table, so a bare name would
/// mislabel same-named FKs on unrelated tables.
#[derive(Debug, Default)]
pub(crate) struct JunctionDetection {
    pub tables: HashSet<String>,
    pub participating_constraints: HashSet<(String, String)>,
}

impl JunctionDetection {
    pub(crate) fn is_participant(&self, table: &str, constraint: &str) -> bool {
        self.participating_constraints
            .contains(&(table.to_string(), constraint.to_string()))
    }
}

/// Detect Junction Table Cards.
///
/// A table is a junction when one of its identity column sets (the key
/// columns of a unique, non-partial, non-expression index — primary
/// keys included) is fully covered by its outgoing FK columns AND those
/// identity columns span at least two distinct FK constraints, none of
/// which covers the identity alone. A single FK covering the whole
/// identity is a shared-key one-to-one child (class-table inheritance),
/// not a many-to-many pairing. The real FK Relationship Edges stay on
/// the map; edges whose columns intersect a detecting identity set are
/// marked as many-to-many participants.
pub(crate) fn detect_junction_tables(
    outgoing: &[OutgoingFk<'_>],
    unique_sets_by_table: &HashMap<String, Vec<Vec<String>>>,
) -> JunctionDetection {
    let mut fks_by_table: HashMap<&str, Vec<&OutgoingFk<'_>>> = HashMap::new();
    for fk in outgoing {
        fks_by_table.entry(fk.table).or_default().push(fk);
    }

    let mut detection = JunctionDetection::default();
    for (table, fks) in &fks_by_table {
        if fks.len() < 2 {
            continue;
        }
        let Some(identity_sets) = unique_sets_by_table.get(*table) else {
            continue;
        };
        for identity in identity_sets {
            if identity.is_empty() {
                continue;
            }
            let covered = identity
                .iter()
                .all(|column| fks.iter().any(|fk| fk.columns.iter().any(|c| c == column)));
            if !covered {
                continue;
            }
            let spanning: Vec<&OutgoingFk<'_>> = fks
                .iter()
                .filter(|fk| fk.columns.iter().any(|c| identity.contains(c)))
                .copied()
                .collect();
            // A spanning FK that covers the identity by itself makes
            // this identity a one-to-one key, not a junction pairing.
            let single_fk_covers_identity = spanning.iter().any(|fk| {
                identity
                    .iter()
                    .all(|column| fk.columns.iter().any(|c| c == column))
            });
            if single_fk_covers_identity {
                continue;
            }
            let spanning_constraints: HashSet<&str> =
                spanning.iter().map(|fk| fk.constraint).collect();
            if spanning_constraints.len() < 2 {
                continue;
            }
            detection.tables.insert((*table).to_string());
            detection.participating_constraints.extend(
                spanning_constraints
                    .into_iter()
                    .map(|constraint| ((*table).to_string(), constraint.to_string())),
            );
        }
    }
    detection
}

/// Whether any referencing (FK) column is nullable. `from_columns_not_null`
/// comes from `pg_attribute.attnotnull` aligned with `from_columns`; a
/// length mismatch means the metadata is unreliable, which we surface
/// as nullable rather than overpromising exactly-one.
pub(crate) fn fk_columns_nullable(from_columns: &[String], from_columns_not_null: &[bool]) -> bool {
    !from_columns.is_empty()
        && (from_columns_not_null.len() != from_columns.len()
            || from_columns_not_null.iter().any(|not_null| !not_null))
}

// ---------------------------------------------------------------------------
// pg_trigger.tgtype decoding
// ---------------------------------------------------------------------------

const TRIGGER_TYPE_ROW: i16 = 1 << 0;
const TRIGGER_TYPE_BEFORE: i16 = 1 << 1;
const TRIGGER_TYPE_INSERT: i16 = 1 << 2;
const TRIGGER_TYPE_DELETE: i16 = 1 << 3;
const TRIGGER_TYPE_UPDATE: i16 = 1 << 4;
const TRIGGER_TYPE_TRUNCATE: i16 = 1 << 5;
const TRIGGER_TYPE_INSTEAD: i16 = 1 << 6;

pub(crate) fn trigger_timing(tgtype: i16) -> &'static str {
    if tgtype & TRIGGER_TYPE_INSTEAD != 0 {
        "INSTEAD OF"
    } else if tgtype & TRIGGER_TYPE_BEFORE != 0 {
        "BEFORE"
    } else {
        "AFTER"
    }
}

pub(crate) fn trigger_events(tgtype: i16) -> Vec<String> {
    let mut events = Vec::new();
    if tgtype & TRIGGER_TYPE_INSERT != 0 {
        events.push("INSERT".to_string());
    }
    if tgtype & TRIGGER_TYPE_UPDATE != 0 {
        events.push("UPDATE".to_string());
    }
    if tgtype & TRIGGER_TYPE_DELETE != 0 {
        events.push("DELETE".to_string());
    }
    if tgtype & TRIGGER_TYPE_TRUNCATE != 0 {
        events.push("TRUNCATE".to_string());
    }
    events
}

pub(crate) fn trigger_orientation(tgtype: i16) -> &'static str {
    if tgtype & TRIGGER_TYPE_ROW != 0 {
        "ROW"
    } else {
        "STATEMENT"
    }
}

/// `pg_trigger.tgenabled`: `O` (origin) / `R` (replica) / `A` (always)
/// fire in at least one session mode; only `D` is disabled outright.
pub(crate) fn trigger_enabled(tgenabled: &str) -> bool {
    tgenabled != "D"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    // -- cardinality classification --------------------------------------

    #[test]
    fn relationship_cardinality_is_one_to_one_for_unique_fk_columns() {
        let (cardinality, reason) = classify_cardinality(true);
        assert_eq!(cardinality, "one-to-one");
        assert!(reason.contains("unique"));
    }

    #[test]
    fn relationship_cardinality_is_one_to_many_for_non_unique_fk_columns() {
        let (cardinality, reason) = classify_cardinality(false);
        assert_eq!(cardinality, "one-to-many");
        assert!(reason.contains("not constrained unique"));
    }

    // -- FK column uniqueness ---------------------------------------------

    #[test]
    fn unique_set_covers_exact_match() {
        assert!(unique_set_covers(
            &cols(&["user_id"]),
            &[cols(&["user_id"])]
        ));
    }

    #[test]
    fn unique_set_covers_subset_of_composite_fk() {
        // unique(user_id) makes (user_id, role_id) unique too.
        assert!(unique_set_covers(
            &cols(&["user_id", "role_id"]),
            &[cols(&["user_id"])]
        ));
    }

    #[test]
    fn unique_set_does_not_cover_superset_of_fk() {
        // unique(user_id, tenant_id) says nothing about user_id alone.
        assert!(!unique_set_covers(
            &cols(&["user_id"]),
            &[cols(&["user_id", "tenant_id"])]
        ));
    }

    #[test]
    fn unique_set_covers_ignores_unrelated_indexes() {
        assert!(!unique_set_covers(
            &cols(&["user_id"]),
            &[cols(&["email"]), cols(&["slug"])]
        ));
    }

    #[test]
    fn unique_set_covers_is_false_without_metadata() {
        assert!(!unique_set_covers(&cols(&["user_id"]), &[]));
        assert!(!unique_set_covers(&[], &[cols(&["user_id"])]));
    }

    // -- junction detection -------------------------------------------------

    fn unique_sets(entries: &[(&str, &[&[&str]])]) -> HashMap<String, Vec<Vec<String>>> {
        entries
            .iter()
            .map(|(table, sets)| {
                (
                    table.to_string(),
                    sets.iter().map(|set| cols(set)).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn junction_detected_for_composite_pk_of_two_fks() {
        let order_cols = cols(&["order_id"]);
        let product_cols = cols(&["product_id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "oi_order_fkey",
                table: "order_items",
                columns: &order_cols,
            },
            OutgoingFk {
                constraint: "oi_product_fkey",
                table: "order_items",
                columns: &product_cols,
            },
        ];
        let uniques = unique_sets(&[("order_items", &[&["order_id", "product_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.contains("order_items"));
        assert!(detection.is_participant("order_items", "oi_order_fkey"));
        assert!(detection.is_participant("order_items", "oi_product_fkey"));
    }

    #[test]
    fn junction_detected_for_surrogate_pk_with_unique_pair() {
        // id PK + unique(user_id, group_id): identity set (user_id,
        // group_id) is FK-covered and spans two constraints.
        let user_cols = cols(&["user_id"]);
        let group_cols = cols(&["group_id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "m_user_fkey",
                table: "memberships",
                columns: &user_cols,
            },
            OutgoingFk {
                constraint: "m_group_fkey",
                table: "memberships",
                columns: &group_cols,
            },
        ];
        let uniques = unique_sets(&[("memberships", &[&["id"], &["user_id", "group_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.contains("memberships"));
        assert_eq!(detection.participating_constraints.len(), 2);
    }

    #[test]
    fn junction_excludes_extra_fk_outside_identity_set() {
        // created_by → users is a real FK on the junction table but not
        // part of the many-to-many path.
        let order_cols = cols(&["order_id"]);
        let product_cols = cols(&["product_id"]);
        let creator_cols = cols(&["created_by"]);
        let outgoing = [
            OutgoingFk {
                constraint: "oi_order_fkey",
                table: "order_items",
                columns: &order_cols,
            },
            OutgoingFk {
                constraint: "oi_product_fkey",
                table: "order_items",
                columns: &product_cols,
            },
            OutgoingFk {
                constraint: "oi_creator_fkey",
                table: "order_items",
                columns: &creator_cols,
            },
        ];
        let uniques = unique_sets(&[("order_items", &[&["order_id", "product_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.contains("order_items"));
        assert!(!detection.is_participant("order_items", "oi_creator_fkey"));
    }

    #[test]
    fn junction_participation_is_keyed_by_table_not_constraint_name_alone() {
        // PG constraint names are unique per table only: an unrelated
        // table reusing a participating FK's name must stay unmarked.
        let user_cols = cols(&["user_id"]);
        let group_cols = cols(&["group_id"]);
        let audit_cols = cols(&["user_id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "fk_user",
                table: "memberships",
                columns: &user_cols,
            },
            OutgoingFk {
                constraint: "fk_group",
                table: "memberships",
                columns: &group_cols,
            },
            OutgoingFk {
                constraint: "fk_user",
                table: "audit_log",
                columns: &audit_cols,
            },
        ];
        let uniques = unique_sets(&[("memberships", &[&["user_id", "group_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.is_participant("memberships", "fk_user"));
        assert!(!detection.is_participant("audit_log", "fk_user"));
    }

    #[test]
    fn shared_pk_one_to_one_child_is_not_a_junction() {
        // Class-table inheritance: cars(id PK, FK(id)→vehicles,
        // FK(id)→assets). Each FK covers the identity alone, so this is
        // a pair of one-to-one edges, not a many-to-many pairing.
        let id_cols = cols(&["id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "cars_vehicle_fkey",
                table: "cars",
                columns: &id_cols,
            },
            OutgoingFk {
                constraint: "cars_asset_fkey",
                table: "cars",
                columns: &id_cols,
            },
        ];
        let uniques = unique_sets(&[("cars", &[&["id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.is_empty());
        assert!(detection.participating_constraints.is_empty());
    }

    #[test]
    fn identity_covered_alone_by_one_unique_fk_is_not_a_junction() {
        // unique(a, b) with FK1(a, b) (a one-to-one edge) plus FK2(b):
        // FK1 covers the identity by itself, so no junction.
        let ab_cols = cols(&["a", "b"]);
        let b_cols = cols(&["b"]);
        let outgoing = [
            OutgoingFk {
                constraint: "t_ab_fkey",
                table: "t",
                columns: &ab_cols,
            },
            OutgoingFk {
                constraint: "t_b_fkey",
                table: "t",
                columns: &b_cols,
            },
        ];
        let uniques = unique_sets(&[("t", &[&["a", "b"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.is_empty());
        assert!(detection.participating_constraints.is_empty());
    }

    #[test]
    fn one_to_one_child_with_second_fk_is_not_a_junction() {
        // profile(user_id PK FK→users, country_id FK→countries): the
        // identity set {user_id} spans only one constraint.
        let user_cols = cols(&["user_id"]);
        let country_cols = cols(&["country_id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "p_user_fkey",
                table: "profiles",
                columns: &user_cols,
            },
            OutgoingFk {
                constraint: "p_country_fkey",
                table: "profiles",
                columns: &country_cols,
            },
        ];
        let uniques = unique_sets(&[("profiles", &[&["user_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.is_empty());
        assert!(detection.participating_constraints.is_empty());
    }

    #[test]
    fn table_with_single_fk_is_not_a_junction() {
        let order_cols = cols(&["order_id"]);
        let outgoing = [OutgoingFk {
            constraint: "oi_order_fkey",
            table: "order_items",
            columns: &order_cols,
        }];
        let uniques = unique_sets(&[("order_items", &[&["order_id"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.is_empty());
    }

    #[test]
    fn junction_requires_identity_fully_covered_by_fk_columns() {
        // PK (order_id, line_no): line_no is not an FK column, so the
        // identity is not FK-covered.
        let order_cols = cols(&["order_id"]);
        let product_cols = cols(&["product_id"]);
        let outgoing = [
            OutgoingFk {
                constraint: "oi_order_fkey",
                table: "order_lines",
                columns: &order_cols,
            },
            OutgoingFk {
                constraint: "oi_product_fkey",
                table: "order_lines",
                columns: &product_cols,
            },
        ];
        let uniques = unique_sets(&[("order_lines", &[&["order_id", "line_no"]])]);

        let detection = detect_junction_tables(&outgoing, &uniques);
        assert!(detection.tables.is_empty());
    }

    // -- FK column nullability ----------------------------------------------

    #[test]
    fn fk_columns_nullable_when_any_column_is_nullable() {
        assert!(fk_columns_nullable(
            &cols(&["user_id", "note_id"]),
            &[true, false]
        ));
    }

    #[test]
    fn fk_columns_not_nullable_when_every_column_is_not_null() {
        assert!(!fk_columns_nullable(
            &cols(&["user_id", "note_id"]),
            &[true, true]
        ));
    }

    #[test]
    fn fk_columns_nullable_on_metadata_length_mismatch() {
        // Unreliable attnotnull metadata must degrade to nullable
        // (zero-or-one) instead of overpromising exactly-one.
        assert!(fk_columns_nullable(&cols(&["user_id", "note_id"]), &[true]));
    }

    #[test]
    fn fk_columns_nullable_is_false_for_empty_fk() {
        assert!(!fk_columns_nullable(&[], &[]));
    }

    // -- trigger decoding ---------------------------------------------------

    #[test]
    fn trigger_timing_decodes_before_after_and_instead_of() {
        assert_eq!(trigger_timing(TRIGGER_TYPE_BEFORE), "BEFORE");
        assert_eq!(trigger_timing(0), "AFTER");
        assert_eq!(
            trigger_timing(TRIGGER_TYPE_INSTEAD | TRIGGER_TYPE_BEFORE),
            "INSTEAD OF"
        );
    }

    #[test]
    fn trigger_events_decode_each_bit_in_stable_order() {
        let all =
            TRIGGER_TYPE_INSERT | TRIGGER_TYPE_UPDATE | TRIGGER_TYPE_DELETE | TRIGGER_TYPE_TRUNCATE;
        assert_eq!(
            trigger_events(all),
            vec!["INSERT", "UPDATE", "DELETE", "TRUNCATE"]
        );
        assert_eq!(trigger_events(TRIGGER_TYPE_UPDATE), vec!["UPDATE"]);
        assert!(trigger_events(0).is_empty());
    }

    #[test]
    fn trigger_orientation_decodes_row_and_statement() {
        assert_eq!(trigger_orientation(TRIGGER_TYPE_ROW), "ROW");
        assert_eq!(trigger_orientation(0), "STATEMENT");
    }

    #[test]
    fn trigger_enabled_treats_only_disabled_as_off() {
        assert!(trigger_enabled("O"));
        assert!(trigger_enabled("R"));
        assert!(trigger_enabled("A"));
        assert!(!trigger_enabled("D"));
    }

    // A BEFORE UPDATE OF ... FOR EACH ROW trigger as PG encodes it.
    #[test]
    fn trigger_decoding_composes_for_a_real_before_update_row_trigger() {
        let tgtype = TRIGGER_TYPE_ROW | TRIGGER_TYPE_BEFORE | TRIGGER_TYPE_UPDATE;
        assert_eq!(trigger_timing(tgtype), "BEFORE");
        assert_eq!(trigger_events(tgtype), vec!["UPDATE"]);
        assert_eq!(trigger_orientation(tgtype), "ROW");
    }
}

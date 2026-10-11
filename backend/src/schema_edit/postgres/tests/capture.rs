use super::super::capture::*;
use super::super::catalog::{attribute_bits, declared_type};
use super::super::planner::preserved_attributes;
use crate::postgres::native_catalog::table_structure::*;
use crate::schema_edit::*;
use std::collections::HashSet;

const RELATION: u32 = 16384;

fn column(number: i32, name: &str, data_type: &str, nullable: bool) -> StructureColumn {
    StructureColumn {
        number,
        name: name.into(),
        data_type: data_type.into(),
        nullable,
        default_expression: None,
        comment: None,
        identity: StructureIdentityKind::None,
        generated: StructureGeneratedKind::None,
        primary_key_position: None,
        collation_schema: None,
        collation_name: None,
    }
}

fn key(position: u16, number: Option<i32>, definition: &str, included: bool) -> StructureIndexKey {
    StructureIndexKey {
        position,
        column_number: number,
        column_name: number.map(|_| definition.trim_matches('"').to_owned()),
        definition: definition.into(),
        included,
    }
}

fn index(oid: u32, name: &str, keys: Vec<StructureIndexKey>, definition: &str) -> StructureIndex {
    StructureIndex {
        oid,
        name: name.into(),
        method: "btree".into(),
        unique: false,
        primary: false,
        valid: true,
        ready: true,
        keys,
        predicate: None,
        definition: definition.into(),
        constraint_oid: None,
    }
}

fn constraint(oid: u32, name: &str, kind: &str, definition: &str) -> StructureConstraint {
    StructureConstraint {
        oid,
        name: name.into(),
        kind: kind.into(),
        definition: definition.into(),
        deferrable: false,
        initially_deferred: false,
        validated: true,
    }
}

fn foreign_key(oid: u32, name: &str) -> StructureForeignKey {
    StructureForeignKey {
        oid,
        name: name.into(),
        source_oid: RELATION,
        source_schema: "public".into(),
        source_table: "users".into(),
        target_oid: 16500,
        target_schema: "public".into(),
        target_table: "orgs".into(),
        columns: vec![StructureKeyPair {
            source_number: 4,
            source: "org_id".into(),
            target_number: 1,
            target: "id".into(),
        }],
        on_update: StructureReferentialAction::NoAction,
        on_delete: StructureReferentialAction::Cascade,
        match_type: "SIMPLE".into(),
        deferrable: false,
        initially_deferred: false,
        validated: true,
    }
}

/// `users(id bigint identity PK, email varchar(100) UNIQUE, name text
/// COLLATE "C" DEFAULT 'anon', org_id → orgs, total generated)`, a check, an
/// expression index, a partial index with INCLUDE, an inbound foreign key
/// and a trigger.
fn snapshot() -> TableStructureSnapshot {
    let mut id = column(1, "id", "bigint", false);
    id.identity = StructureIdentityKind::ByDefault;
    id.primary_key_position = Some(1);
    let mut email = column(2, "email", "character varying(100)", false);
    email.collation_schema = Some("pg_catalog".into());
    email.collation_name = Some("default".into());
    let mut name = column(3, "name", "text", true);
    name.default_expression = Some("'anon'::text".into());
    name.collation_schema = Some("pg_catalog".into());
    name.collation_name = Some("C".into());
    name.comment = Some("Display name".into());
    let org = column(4, "org_id", "bigint", true);
    let mut total = column(5, "total", "bigint", true);
    total.generated = StructureGeneratedKind::Stored;
    total.default_expression = Some("(id * 2)".into());

    let mut pkey = index(
        30001,
        "users_pkey",
        vec![key(1, Some(1), "id", false)],
        "CREATE UNIQUE INDEX users_pkey ON public.users USING btree (id)",
    );
    pkey.unique = true;
    pkey.primary = true;
    pkey.constraint_oid = Some(20001);
    let mut email_key = index(
        30002,
        "users_email_key",
        vec![key(1, Some(2), "email", false)],
        "CREATE UNIQUE INDEX users_email_key ON public.users USING btree (email)",
    );
    email_key.unique = true;
    email_key.constraint_oid = Some(20002);
    let lower = index(
        30003,
        "users_lower_email_idx",
        vec![key(1, None, "lower(email::text)", false)],
        "CREATE INDEX users_lower_email_idx ON public.users USING btree (lower((email)::text) DESC)",
    );
    let mut partial = index(
        30004,
        "users_name_idx",
        vec![key(1, Some(3), "name", false), key(2, Some(4), "org_id", true)],
        "CREATE INDEX users_name_idx ON public.users USING btree (name) INCLUDE (org_id) WHERE (org_id IS NOT NULL)",
    );
    partial.predicate = Some("(org_id IS NOT NULL)".into());

    let mut inbound = foreign_key(20010, "posts_author_fkey");
    inbound.source_oid = 17000;
    inbound.source_table = "posts".into();
    inbound.target_oid = RELATION;
    inbound.target_table = "users".into();

    TableStructureSnapshot {
        identity: TableIdentity {
            database_oid: 1,
            relation_oid: RELATION,
        },
        schema: "public".into(),
        table: "users".into(),
        kind: StructureRelationKind::Table,
        owner: "app".into(),
        comment: Some("People".into()),
        server_version: 160_004,
        captured_at: "2026-10-12 00:00:00+00".into(),
        columns: vec![id, email, name, org, total],
        primary_key: Some(StructurePrimaryKey {
            oid: 20001,
            name: "users_pkey".into(),
            columns: vec![StructureKeyColumn {
                number: 1,
                name: "id".into(),
            }],
            deferrable: false,
            initially_deferred: false,
        }),
        outbound: vec![foreign_key(20003, "users_org_id_fkey")],
        inbound: vec![inbound],
        indexes: vec![pkey, email_key, lower, partial],
        constraints: vec![
            constraint(20004, "users_email_check", "check", "CHECK (length(email::text) > 3)"),
            constraint(20002, "users_email_key", "unique", "UNIQUE (email)"),
            constraint(20005, "users_id_not_null", "not null", "NOT NULL id"),
            constraint(
                20003,
                "users_org_id_fkey",
                "foreign key",
                "FOREIGN KEY (org_id) REFERENCES orgs(id) ON DELETE CASCADE",
            ),
            constraint(20001, "users_pkey", "primary key", "PRIMARY KEY (id)"),
        ],
        triggers: vec![StructureTrigger {
            oid: 40001,
            name: "users_audit".into(),
            parent_trigger_oid: None,
            timing: "AFTER".into(),
            events: vec!["UPDATE".into()],
            update_columns: Vec::new(),
            level: "ROW".into(),
            enabled: StructureTriggerEnabled::Origin,
            function_oid: 50001,
            function_schema: "public".into(),
            function_name: "audit".into(),
            definition: "CREATE TRIGGER users_audit AFTER UPDATE ON public.users FOR EACH ROW EXECUTE FUNCTION audit()".into(),
        }],
        row_security: StructureRowSecurity {
            enabled: false,
            forced: false,
        },
        policies: Vec::new(),
        privileges: Vec::new(),
        rules: Vec::new(),
        partition_key: None,
        is_partition: false,
        partition_bound: None,
        parents: Vec::new(),
        partitions: Vec::new(),
    }
}

fn baseline(snapshot: &TableStructureSnapshot) -> SchemaBaseline {
    baseline_from_snapshot(snapshot, drift_token(RELATION, "abc"), Some(42))
}

fn reasons(snapshot: &TableStructureSnapshot) -> Vec<String> {
    match baseline(snapshot).editable {
        Editability::ReadOnly { reasons } => reasons,
        Editability::Editable => Vec::new(),
    }
}

fn read_only_because(snapshot: &TableStructureSnapshot, text: &str) {
    let reasons = reasons(snapshot);
    assert!(
        reasons.iter().any(|reason| reason.contains(text)),
        "{text:?} not in {reasons:?}"
    );
}

fn col(n: u64) -> Origin {
    Origin::Existing(BaselineId(n))
}

#[test]
fn converts_a_complete_table() {
    let baseline = baseline(&snapshot());
    assert_eq!(baseline.editable, Editability::Editable);
    assert_eq!(baseline.engine, SchemaEngine::Postgres);
    assert_eq!(baseline.server.version, "16.4");
    assert!(baseline.server.features.transactional_ddl);
    assert!(baseline.server.features.rename_column);
    assert!(!baseline.server.features.column_reorder);
    assert_eq!(baseline.estimated_rows, Some(42));
    assert_eq!(drift_oid(&baseline.drift), Some(RELATION));

    let table = &baseline.table;
    assert_eq!(table.comment.as_deref(), Some("People"));
    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "email", "name", "org_id", "total"]);
    let id = table.column(col(1)).unwrap();
    assert_eq!(id.identity, Identity::ByDefault);
    assert_eq!(id.default, DefaultValue::None);
    assert!(!id.nullable);
    let email = table.column(col(2)).unwrap();
    assert_eq!(email.data_type, "character varying(100)");
    assert_eq!(email.collation, None, "the default collation is not listed");
    let name = table.column(col(3)).unwrap();
    assert_eq!(
        name.default,
        DefaultValue::Expression(SqlFragment::new("'anon'::text"))
    );
    assert_eq!(name.collation.as_deref(), Some("C"));
    assert_eq!(name.comment.as_deref(), Some("Display name"));
    let total = table.column(col(5)).unwrap();
    assert_eq!(total.default, DefaultValue::None);
    assert_eq!(
        total.generated,
        Some(Generated {
            expression: SqlFragment::new("(id * 2)"),
            stored: true
        })
    );

    assert_eq!(
        table.primary_key,
        Some(KeyModel {
            origin: object_origin(TAG_PRIMARY_KEY, 20001),
            name: Some("users_pkey".into()),
            columns: vec![col(1)],
        })
    );
    assert_eq!(
        table.uniques,
        vec![KeyModel {
            origin: object_origin(TAG_UNIQUE, 20002),
            name: Some("users_email_key".into()),
            columns: vec![col(2)],
        }]
    );
    assert_eq!(
        table.checks,
        vec![CheckModel {
            origin: object_origin(TAG_CHECK, 20004),
            name: Some("users_email_check".into()),
            expression: SqlFragment::new("length(email::text) > 3"),
            enforced: true,
        }]
    );
    assert_eq!(
        table.foreign_keys,
        vec![ForeignKeyModel {
            origin: object_origin(TAG_FOREIGN_KEY, 20003),
            name: Some("users_org_id_fkey".into()),
            columns: vec![col(4)],
            referenced_schema: None,
            referenced_table: "orgs".into(),
            referenced_columns: vec!["id".into()],
            on_update: ReferentialAction::NoAction,
            on_delete: ReferentialAction::Cascade,
            deferrable: Deferrable::Never,
        }]
    );
    // Constraint-backed indexes are the constraints, not indexes.
    assert_eq!(
        table.indexes,
        vec![
            IndexModel {
                origin: object_origin(TAG_INDEX, 30003),
                name: "users_lower_email_idx".into(),
                unique: false,
                method: None,
                members: vec![IndexMember {
                    target: IndexTarget::Expression(SqlFragment::new("lower(email::text)")),
                    descending: true,
                    prefix_length: None,
                    collation: None,
                }],
                include: Vec::new(),
                predicate: None,
            },
            IndexModel {
                origin: object_origin(TAG_INDEX, 30004),
                name: "users_name_idx".into(),
                unique: false,
                method: None,
                members: vec![IndexMember::column(col(3))],
                include: vec![col(4)],
                predicate: Some(SqlFragment::new("(org_id IS NOT NULL)")),
            },
        ]
    );
    assert_eq!(
        baseline.inbound,
        vec![InboundReference {
            schema: "public".into(),
            table: "posts".into(),
            name: "posts_author_fkey".into(),
            columns: vec!["id".into()],
        }]
    );
    assert_eq!(baseline.dependents.len(), 1);
    assert_eq!(baseline.dependents[0].kind, "trigger");
    assert_eq!(baseline.dependents[0].name, "users_audit");
}

#[test]
fn ids_are_stable_and_disjoint_across_kinds() {
    let snapshot = snapshot();
    let first = baseline(&snapshot);
    assert_eq!(first, baseline(&snapshot));
    let table = &first.table;
    let origins: Vec<Origin> = table
        .columns
        .iter()
        .map(|c| c.origin)
        .chain(table.primary_key.iter().map(|k| k.origin))
        .chain(table.uniques.iter().map(|k| k.origin))
        .chain(table.foreign_keys.iter().map(|k| k.origin))
        .chain(table.checks.iter().map(|c| c.origin))
        .chain(table.indexes.iter().map(|i| i.origin))
        .collect();
    let unique: HashSet<Origin> = origins.iter().copied().collect();
    assert_eq!(unique.len(), origins.len());
    // The same OID under two kinds stays distinct.
    assert_ne!(object_origin(TAG_CHECK, 7), object_origin(TAG_INDEX, 7));
    assert_ne!(object_origin(TAG_CHECK, 7), col(7));
}

#[test]
fn an_untouched_draft_of_a_capture_plans_nothing() {
    let baseline = baseline(&snapshot());
    let draft = SchemaDraft::from_baseline(&baseline);
    assert!(diff(&baseline, &draft).is_empty());
    assert!(validate(&baseline, &draft).is_empty());
    assert_eq!(
        plan_review(&baseline, &draft, &PlanOptions::default()),
        Err(PlanError::NoChanges)
    );
}

#[test]
fn relations_the_model_cannot_edit_are_read_only() {
    let mut partitioned = snapshot();
    partitioned.kind = StructureRelationKind::PartitionedTable;
    read_only_because(&partitioned, "Partitioned tables");

    let mut view = snapshot();
    view.kind = StructureRelationKind::View;
    read_only_because(&view, "Views");

    let mut partition = snapshot();
    partition.is_partition = true;
    read_only_because(&partition, "partition");

    let relative = StructureRelative {
        oid: 9,
        schema: "public".into(),
        name: "base".into(),
        sequence: 1,
        is_partition: false,
        bound: None,
    };
    let mut child = snapshot();
    child.parents.push(relative.clone());
    read_only_because(&child, "inherits");

    let mut parent = snapshot();
    parent.partitions.push(relative);
    read_only_because(&parent, "inherit from this table");
}

#[test]
fn constraints_the_model_cannot_represent_are_read_only() {
    let mut deferrable = snapshot();
    let unique = deferrable
        .constraints
        .iter_mut()
        .find(|c| c.kind == "unique")
        .unwrap();
    unique.deferrable = true;
    unique.definition = "UNIQUE (email) DEFERRABLE".into();
    read_only_because(&deferrable, "deferrable");

    let mut included = snapshot();
    included
        .constraints
        .iter_mut()
        .find(|c| c.kind == "unique")
        .unwrap()
        .definition = "UNIQUE (email) INCLUDE (name)".into();
    read_only_because(&included, "Key users_email_key has options");

    let mut exclusion = snapshot();
    exclusion.constraints.push(constraint(
        20006,
        "users_no_overlap",
        "exclusion",
        "EXCLUDE USING gist (name WITH =)",
    ));
    read_only_because(&exclusion, "Exclusion constraint users_no_overlap");

    let mut not_valid = snapshot();
    let check = not_valid
        .constraints
        .iter_mut()
        .find(|c| c.kind == "check")
        .unwrap();
    check.validated = false;
    check.definition = "CHECK (length(email::text) > 3) NOT VALID".into();
    read_only_because(&not_valid, "NOT VALID");

    let mut no_inherit = snapshot();
    no_inherit
        .constraints
        .iter_mut()
        .find(|c| c.kind == "check")
        .unwrap()
        .definition = "CHECK (length(email::text) > 3) NO INHERIT".into();
    read_only_because(&no_inherit, "Check users_email_check has options");

    let mut full = snapshot();
    full.outbound[0].match_type = "FULL".into();
    read_only_because(&full, "MATCH FULL");

    let mut set_null_columns = snapshot();
    set_null_columns
        .constraints
        .iter_mut()
        .find(|c| c.kind == "foreign key")
        .unwrap()
        .definition = "FOREIGN KEY (org_id) REFERENCES orgs(id) ON DELETE SET NULL (org_id)".into();
    read_only_because(
        &set_null_columns,
        "Foreign key users_org_id_fkey has options",
    );

    let mut temporal = snapshot();
    temporal
        .constraints
        .iter_mut()
        .find(|c| c.kind == "foreign key")
        .unwrap()
        .definition =
        "FOREIGN KEY (org_id, PERIOD valid_at) REFERENCES orgs(id, PERIOD valid_at)".into();
    read_only_because(&temporal, "temporal (PERIOD) foreign key");

    let mut collation = snapshot();
    collation.columns[2].collation_schema = Some("public".into());
    collation.columns[2].collation_name = Some("german".into());
    read_only_because(&collation, "outside pg_catalog");
}

#[test]
fn indexes_the_model_cannot_represent_are_read_only() {
    let mut invalid = snapshot();
    invalid.indexes[2].valid = false;
    read_only_because(&invalid, "is invalid");

    for definition in [
        "CREATE INDEX users_name_idx ON public.users USING btree (name text_pattern_ops) INCLUDE (org_id) WHERE (org_id IS NOT NULL)",
        "CREATE INDEX users_name_idx ON public.users USING btree (name COLLATE \"C\") INCLUDE (org_id) WHERE (org_id IS NOT NULL)",
        "CREATE INDEX users_name_idx ON public.users USING btree (name NULLS FIRST) INCLUDE (org_id) WHERE (org_id IS NOT NULL)",
        "CREATE INDEX users_name_idx ON public.users USING btree (name DESC NULLS LAST) INCLUDE (org_id) WHERE (org_id IS NOT NULL)",
        "CREATE INDEX users_name_idx ON public.users USING btree (name) INCLUDE (org_id) WITH (fillfactor='70') WHERE (org_id IS NOT NULL)",
        "CREATE INDEX users_name_idx ON public.users USING btree (name)",
    ] {
        let mut options = snapshot();
        options.indexes[3].definition = definition.into();
        read_only_because(&options, "users_name_idx uses options");
    }

    let mut opclass_expression = snapshot();
    opclass_expression.indexes[2].definition =
        "CREATE INDEX users_lower_email_idx ON public.users USING btree (lower((email)::text) text_pattern_ops)".into();
    read_only_because(&opclass_expression, "users_lower_email_idx uses options");
}

#[test]
fn non_btree_methods_and_quoted_names_are_kept() {
    let mut snapshot = snapshot();
    snapshot.columns[2].name = "Full Name".into();
    let gin = StructureIndex {
        method: "gin".into(),
        ..index(
            30005,
            "users_full_name_idx",
            vec![key(1, Some(3), "\"Full Name\"", false)],
            "CREATE INDEX users_full_name_idx ON public.users USING gin (\"Full Name\")",
        )
    };
    snapshot.indexes.push(gin);
    let baseline = baseline(&snapshot);
    assert_eq!(baseline.editable, Editability::Editable);
    let index = baseline.table.indexes.last().unwrap();
    assert_eq!(index.method.as_deref(), Some("gin"));
    assert_eq!(index.members, vec![IndexMember::column(col(3))]);
}

#[test]
fn helpers() {
    assert_eq!(server_version_text(160_004), "16.4");
    assert_eq!(server_version_text(130_012), "13.12");
    assert_eq!(drift_oid(&drift_token(42, "ff")), Some(42));
    assert_eq!(drift_oid(&DriftToken("fixture".into())), None);
    assert_eq!(estimated_rows(-1.0), None);
    assert_eq!(estimated_rows(1234.0), Some(1234));
    assert_eq!(check_expression("CHECK (a > 0)"), Some("a > 0"));
    assert_eq!(
        check_expression("CHECK ((a > 0) AND (b > 0))"),
        Some("(a > 0) AND (b > 0)")
    );
    assert_eq!(check_expression("CHECK (a > 0) NO INHERIT"), None);
    assert_eq!(check_expression("CHECK (a > ')') NOT VALID"), None);
    assert_eq!(strip_outer_parens("(a + b)"), "a + b");
    assert_eq!(strip_outer_parens("(a) + (b)"), "(a) + (b)");
    assert_eq!(strip_outer_parens("lower(a)"), "lower(a)");
}

#[test]
fn preserved_attributes_mark_only_the_objects_that_carry_them() {
    let snapshot = snapshot();
    let mut baseline = baseline(&snapshot);
    let comment = attribute_bits([false, false, true, false, false, false]);
    let replica_and_stats = attribute_bits([false, false, false, true, false, true]);
    assert_eq!(comment, 1 << 34);
    assert_eq!(replica_and_stats, (1 << 35) | (1 << 37));
    assert_eq!(attribute_bits([false; 6]), 0);

    let attributes = PreservedAttributes {
        // The unique key (by constraint OID) and the expression index (by
        // index OID); index OIDs never mark constraints, nor the reverse.
        constraints: [(20002, comment)].into_iter().collect(),
        indexes: [(30003, replica_and_stats), (20004, comment)]
            .into_iter()
            .collect(),
    };
    mark_preserved(&mut baseline.table, &attributes);
    let table = &baseline.table;
    assert_eq!(
        preserved_attributes(table.uniques[0].origin),
        vec!["a comment"]
    );
    let expression = table
        .indexes
        .iter()
        .find(|index| index.name == "users_lower_email_idx")
        .unwrap();
    assert_eq!(
        preserved_attributes(expression.origin),
        vec!["the replica identity", "column statistics targets"]
    );
    assert!(preserved_attributes(table.checks[0].origin).is_empty());
    assert!(preserved_attributes(table.primary_key.as_ref().unwrap().origin).is_empty());
    assert!(preserved_attributes(table.foreign_keys[0].origin).is_empty());

    // An untouched draft of a marked capture still plans nothing.
    let draft = SchemaDraft::from_baseline(&baseline);
    assert_eq!(
        plan_review(&baseline, &draft, &PlanOptions::default()),
        Err(PlanError::NoChanges)
    );
}

#[test]
fn a_domain_type_resolves_to_the_domain_not_its_base() {
    const VARCHAR: u32 = 1043;
    const DOMAIN: u32 = 70001;
    // A plain type keeps the reported OID and modifier.
    assert_eq!(declared_type((VARCHAR, 24), VARCHAR), (VARCHAR, 24));
    // The wire reports a domain as its base type; the domain wins, unmodified.
    assert_eq!(declared_type((VARCHAR, 24), DOMAIN), (DOMAIN, -1));
}

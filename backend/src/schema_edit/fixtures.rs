//! Test baselines shared by `diff` tests and the engine planners' golden
//! tests. Origins: columns are `Existing(1..)`, the primary key 100, uniques
//! 200+, foreign keys 300+, checks 400+, indexes 500+.
use super::model::*;

pub(crate) fn col(id: u64, name: &str, data_type: &str) -> ColumnModel {
    ColumnModel::new(Origin::Existing(BaselineId(id)), name, data_type)
}

pub(crate) fn existing(id: u64) -> Origin {
    Origin::Existing(BaselineId(id))
}

pub(crate) fn features(engine: SchemaEngine) -> ServerFeatures {
    match engine {
        SchemaEngine::Postgres => ServerFeatures {
            rename_column: true,
            drop_column: true,
            enforced_check: true,
            transactional_ddl: true,
            ..ServerFeatures::default()
        },
        SchemaEngine::MySql => ServerFeatures {
            rename_column: true,
            drop_column: true,
            instant_add_column: true,
            instant_add_column_anywhere: true,
            enforced_check: true,
            column_reorder: true,
            transactional_ddl: false,
        },
        SchemaEngine::Sqlite => ServerFeatures {
            rename_column: true,
            drop_column: true,
            enforced_check: true,
            transactional_ddl: true,
            ..ServerFeatures::default()
        },
    }
}

/// `users(id bigint PK, email varchar(100) NOT NULL UNIQUE, name text,
/// org_id bigint FK → orgs(id), CHECK (length(email) > 3), INDEX on name)`.
pub(crate) fn users(engine: SchemaEngine) -> SchemaBaseline {
    let schema = match engine {
        SchemaEngine::Postgres => "public",
        SchemaEngine::MySql => "app",
        SchemaEngine::Sqlite => "main",
    };
    let mut id = col(1, "id", "bigint");
    id.nullable = false;
    let mut email = col(2, "email", "varchar(100)");
    email.nullable = false;
    let name = col(3, "name", "text");
    let org = col(4, "org_id", "bigint");
    let (flavor, version) = match engine {
        SchemaEngine::Postgres => (ServerFlavor::Postgres, "16.4"),
        SchemaEngine::MySql => (ServerFlavor::MySql, "8.0.36"),
        SchemaEngine::Sqlite => (ServerFlavor::Sqlite, "3.45.1"),
    };
    SchemaBaseline {
        engine,
        server: ServerFacts {
            flavor,
            version: version.into(),
            features: features(engine),
        },
        table: TableModel {
            schema: schema.into(),
            name: "users".into(),
            comment: None,
            columns: vec![id, email, name, org],
            primary_key: Some(KeyModel {
                origin: existing(100),
                name: Some("users_pkey".into()),
                columns: vec![existing(1)],
            }),
            uniques: vec![KeyModel {
                origin: existing(200),
                name: Some("users_email_key".into()),
                columns: vec![existing(2)],
            }],
            foreign_keys: vec![ForeignKeyModel {
                origin: existing(300),
                name: Some("users_org_id_fkey".into()),
                columns: vec![existing(4)],
                referenced_schema: None,
                referenced_table: "orgs".into(),
                referenced_columns: vec!["id".into()],
                on_update: ReferentialAction::NoAction,
                on_delete: ReferentialAction::Cascade,
                deferrable: Deferrable::Never,
            }],
            checks: vec![CheckModel {
                origin: existing(400),
                name: Some("users_email_check".into()),
                expression: SqlFragment::new("length(email) > 3"),
                enforced: true,
            }],
            indexes: vec![IndexModel {
                origin: existing(500),
                name: "users_name_idx".into(),
                unique: false,
                method: None,
                members: vec![IndexMember::column(existing(3))],
                include: Vec::new(),
                predicate: None,
            }],
            options: TableOptions::default(),
        },
        drift: DriftToken("fixture".into()),
        editable: Editability::Editable,
        inbound: Vec::new(),
        dependents: Vec::new(),
        estimated_rows: Some(10),
    }
}

use super::*;
fn snapshot(scope: SchemaMapScope) -> SchemaMapSnapshot {
    SchemaMapSnapshot {
        database: "db".into(),
        database_oid: 42,
        captured_at: "2026-10-03T00:00:00Z".into(),
        server_version: 170000,
        scope,
        schema_oid: Some(9),
        focus: Some(SchemaMapIdentity {
            database_oid: 42,
            relation_oid: 10,
        }),
        tables: vec![],
        foreign_keys: vec![],
    }
}
#[test]
fn reconnect_refresh_preserves_observed_identity_until_explicit_clear() {
    let captured = snapshot(SchemaMapScope::Relation {
        schema: "a.b".into(),
        table: "c".into(),
        expected: None,
    });
    let request = SchemaMapRequest {
        scope: captured.scope.clone(),
        expected_database_oid: None,
    };
    let kept = runtime::preserve_identity(request.clone(), Some(&captured));
    assert_eq!(kept.expected_database_oid, Some(42));
    assert!(matches!(
        kept.scope,
        SchemaMapScope::Relation {
            expected: Some(SchemaMapIdentity {
                relation_oid: 10,
                ..
            }),
            ..
        }
    ));
    assert_eq!(runtime::preserve_identity(request.clone(), None), request);
    let other = SchemaMapRequest {
        scope: SchemaMapScope::Relation {
            schema: "a".into(),
            table: "b.c".into(),
            expected: None,
        },
        expected_database_oid: None,
    };
    assert_eq!(
        runtime::preserve_identity(other.clone(), Some(&captured)),
        other
    );
    let schema = SchemaMapRequest {
        scope: SchemaMapScope::Schema {
            name: "a.b".into(),
            expected_oid: None,
        },
        expected_database_oid: None,
    };
    assert_eq!(
        runtime::preserve_identity(schema.clone(), Some(&captured)),
        schema
    );
}
#[test]
fn replacement_incoming_admission_keeps_previous_lease_and_releases_on_refusal() {
    let budget = Rc::new(Cell::new(125 * 1024 * 1024));
    let previous = Lease::new(budget.clone(), 2 * 1024 * 1024).unwrap();
    assert!(Lease::new(budget.clone(), 2 * 1024 * 1024).is_err());
    assert_eq!(budget.get(), 127 * 1024 * 1024);
    drop(previous);
    assert_eq!(budget.get(), 125 * 1024 * 1024);
    let incoming = Lease::new(budget.clone(), 3 * 1024 * 1024).unwrap();
    assert_eq!(budget.get(), 128 * 1024 * 1024);
    drop(incoming);
    assert_eq!(budget.get(), 125 * 1024 * 1024);
}

#[test]
fn full_stale_positions_refuse_new_geometry_before_a_save_can_claim_success() {
    let mut value = SchemaMapPreferences::for_database(42);
    value.positions = (1..=MAX_MAP_POSITIONS)
        .map(|oid| SavedPosition {
            identity: SchemaMapIdentity {
                database_oid: 42,
                relation_oid: oid as u32,
            },
            position: MapPoint { x: 0., y: 0. },
        })
        .collect();
    let replacement = SchemaMapIdentity {
        database_oid: 42,
        relation_oid: 9000,
    };
    assert!(!movement::position_record_available(&value, replacement));
    assert!(movement::position_record_available(
        &value,
        value.positions[0].identity
    ));
    value.positions.clear();
    assert!(movement::position_record_available(&value, replacement));
}

fn movement_scene(revision: u64) -> (Scene, SchemaMapPreferences, SchemaMapIdentity) {
    let identity = SchemaMapIdentity {
        database_oid: 42,
        relation_oid: 9000,
    };
    let mut capture = snapshot(SchemaMapScope::Database);
    capture.schema_oid = None;
    capture.focus = None;
    capture.tables.push(SchemaMapTable {
        identity,
        schema_oid: 9,
        schema: "s".into(),
        name: "t".into(),
        kind: SchemaMapTableKind::Table,
        external: false,
        junction: false,
        columns: vec![SchemaMapColumn {
            attnum: 1,
            name: "id".into(),
            data_type: "bigint".into(),
            nullable: false,
            primary_key: true,
            comment: None,
        }],
        triggers: vec![],
    });
    let prefs = SchemaMapPreferences::for_database(42);
    let scene = Scene::new(
        Arc::new(capture),
        SceneKey {
            document_generation: 1,
            capture_generation: 2,
            layout_revision: revision,
        },
        prefs.prefs,
        &[],
        Rc::new(Cell::new(0)),
    )
    .unwrap();
    (scene, prefs, identity)
}

#[test]
fn keyboard_world_nudge_records_exact_oid_and_deauthorizes_previous_selection() {
    let (mut scene, mut value, identity) = movement_scene(0);
    let old = scene.nodes()[0].bounds;
    let selected = scene.node_selection(identity).unwrap();
    let next =
        movement::move_recorded_node(&mut scene, &mut value, selected, MapPoint { x: 10., y: 0. })
            .unwrap();
    assert!(!scene.accepts(selected));
    assert!(scene.accepts(next));
    assert_eq!(scene.nodes()[0].bounds.x, old.x + 10.);
    assert_eq!(
        value.positions,
        vec![SavedPosition {
            identity,
            position: MapPoint {
                x: old.x + 10.,
                y: old.y
            },
        }]
    );
    // Keyboard movement is in world units, even when the map is fitted tiny.
    let camera = Camera {
        pan: MapPoint { x: 0., y: 0. },
        zoom: 0.001,
    };
    let old_screen = camera.to_screen(MapPoint { x: old.x, y: old.y }).unwrap();
    let new_screen = camera.to_screen(value.positions[0].position).unwrap();
    assert!((new_screen.x - old_screen.x - 0.01).abs() < 1e-9);
    let before = value.clone();
    let bounds = scene.nodes()[0].bounds;
    assert!(
        movement::move_recorded_node(&mut scene, &mut value, selected, MapPoint { x: 10., y: 0. })
            .is_err()
    );
    assert_eq!(value, before);
    assert_eq!(scene.nodes()[0].bounds, bounds);
}

#[test]
fn refused_movement_preserves_geometry_and_record_for_all_failure_boundaries() {
    let (mut scene, mut value, identity) = movement_scene(0);
    let selected = scene.node_selection(identity).unwrap();
    let key = scene.key();
    let bounds = scene.nodes()[0].bounds;
    value.positions = (1..=MAX_MAP_POSITIONS)
        .map(|oid| SavedPosition {
            identity: SchemaMapIdentity {
                database_oid: 42,
                relation_oid: oid as u32,
            },
            position: MapPoint { x: 0., y: 0. },
        })
        .collect();
    let before = value.clone();
    assert!(
        movement::move_recorded_node(&mut scene, &mut value, selected, MapPoint { x: 10., y: 0. })
            .is_err()
    );
    assert_eq!(value, before);
    assert_eq!(scene.key(), key);
    assert_eq!(scene.nodes()[0].bounds, bounds);
    value.positions.clear();
    let edge = Selection::Edge {
        key,
        identity: crate::schema_map_model::EdgeIdentity {
            database_oid: 42,
            constraint_oid: 100,
        },
    };
    for (selection, delta) in [
        (edge, MapPoint { x: 10., y: 0. }),
        (
            selected,
            MapPoint {
                x: f64::INFINITY,
                y: 0.,
            },
        ),
        (
            selected,
            MapPoint {
                x: MAX_MAP_POSITION_COORDINATE + 1.,
                y: 0.,
            },
        ),
    ] {
        assert!(movement::move_recorded_node(&mut scene, &mut value, selection, delta).is_err());
        assert!(value.positions.is_empty());
        assert_eq!(scene.key(), key);
        assert_eq!(scene.nodes()[0].bounds, bounds);
    }
    let (mut scene, mut value, identity) = movement_scene(u64::MAX);
    let selection = scene.node_selection(identity).unwrap();
    let bounds = scene.nodes()[0].bounds;
    assert!(
        movement::move_recorded_node(
            &mut scene,
            &mut value,
            selection,
            MapPoint { x: 10., y: 0. }
        )
        .is_err()
    );
    assert_eq!(scene.key().layout_revision, u64::MAX);
    assert_eq!(scene.nodes()[0].bounds, bounds);
    assert!(value.positions.is_empty());
}

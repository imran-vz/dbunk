use super::*;
fn id(relation_oid: u32) -> SchemaMapIdentity {
    SchemaMapIdentity {
        database_oid: 1,
        relation_oid,
    }
}
fn key() -> SceneKey {
    SceneKey {
        document_generation: 1,
        capture_generation: 2,
        layout_revision: 3,
    }
}
fn table(oid: u32, name: &str) -> SchemaMapTable {
    SchemaMapTable {
        identity: id(oid),
        schema_oid: 2,
        schema: "public".into(),
        name: name.into(),
        kind: SchemaMapTableKind::Table,
        external: false,
        columns: (1..=3)
            .map(|attnum| SchemaMapColumn {
                attnum,
                name: format!("c{attnum}"),
                data_type: "bigint".into(),
                nullable: false,
                primary_key: attnum == 1,
                comment: None,
            })
            .collect(),
        triggers: vec![],
        junction: false,
    }
}
fn fk(oid: u32, source: u32, target: u32) -> SchemaMapForeignKey {
    SchemaMapForeignKey {
        database_oid: 1,
        constraint_oid: oid,
        name: format!("fk{oid}"),
        source: id(source),
        target: id(target),
        columns: vec![
            SchemaMapColumnPair {
                source: 2,
                target: 1,
            },
            SchemaMapColumnPair {
                source: 1,
                target: 2,
            },
        ],
        on_update: SchemaMapAction::Cascade,
        on_delete: SchemaMapAction::Restrict,
        match_type: "SIMPLE".into(),
        validated: true,
        deferrable: false,
        columns_nullable: false,
        columns_unique: false,
        cardinality: SchemaMapCardinality::OneToMany,
        cardinality_reason: "Referencing columns are not unique".into(),
        junction_participant: false,
    }
}
fn snapshot() -> SchemaMapSnapshot {
    SchemaMapSnapshot {
        database: "db".into(),
        database_oid: 1,
        captured_at: "2026-10-03T10:00:00Z".into(),
        server_version: 170000,
        scope: SchemaMapScope::Database,
        schema_oid: None,
        focus: None,
        tables: vec![
            table(10, "a"),
            table(11, "b"),
            table(12, "c"),
            table(13, "disconnected"),
        ],
        foreign_keys: vec![
            fk(20, 10, 11),
            fk(21, 11, 10),
            fk(22, 11, 12),
            fk(23, 10, 10),
        ],
    }
}
fn scene(s: SchemaMapSnapshot) -> Scene {
    Scene::new(
        Arc::new(s),
        key(),
        MapPrefs::default(),
        &[],
        Rc::new(Cell::new(0)),
    )
    .unwrap()
}
#[test]
fn cyclic_disconnected_layout_is_finite_stable_and_nonoverlapping() {
    let a = scene(snapshot());
    let mut reversed = snapshot();
    reversed.tables.reverse();
    reversed.foreign_keys.reverse();
    let b = scene(reversed);
    assert_eq!(
        a.nodes
            .iter()
            .map(|n| (n.identity, n.bounds))
            .collect::<Vec<_>>(),
        b.nodes
            .iter()
            .map(|n| (n.identity, n.bounds))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        a.edges
            .iter()
            .map(|e| (e.identity, e.path.clone()))
            .collect::<Vec<_>>(),
        b.edges
            .iter()
            .map(|e| (e.identity, e.path.clone()))
            .collect::<Vec<_>>()
    );
    for (i, node) in a.nodes.iter().enumerate() {
        assert!(node.bounds.valid());
        for other in &a.nodes[i + 1..] {
            assert!(!node.bounds.intersects(other.bounds));
        }
    }
    assert_eq!(a.nodes[0].bounds.x, a.nodes[1].bounds.x);
    assert!(a.nodes[2].bounds.x > a.nodes[1].bounds.x);
    assert!(
        a.edges
            .iter()
            .all(|e| e.path.points().iter().copied().all(camera::valid_point))
    );
}
#[test]
fn oid_attnum_anchors_survive_hidden_rows_and_reject_stale_generations() {
    let mut s = snapshot();
    s.tables[0].schema = "a.b".into();
    s.tables[0].name = "c".into();
    s.tables[1].schema = "a".into();
    s.tables[1].name = "b.c".into();
    let a = scene(s);
    assert_ne!(a.anchor(id(10), 2, true), a.anchor(id(11), 2, true));
    assert!(a.anchor(id(10), 99, true).is_none());
    let selection = a.node_selection(id(10)).unwrap();
    let next = SceneKey {
        layout_revision: 4,
        ..key()
    };
    let b = a
        .rebuild(
            next,
            MapPrefs {
                attributes: MapAttributes::None,
                ..MapPrefs::default()
            },
            &[],
        )
        .unwrap();
    assert!(!b.accepts(selection));
    assert!(b.details(selection).is_err());
    let node = b.nodes.iter().find(|n| n.identity == id(10)).unwrap();
    assert_eq!(
        b.anchor(id(10), 2, true).unwrap().y,
        node.bounds.y + HEADER_HEIGHT / 2.
    );
    assert!(
        b.hit_test(
            key(),
            MapPoint {
                x: node.bounds.x + 5.,
                y: node.bounds.y + 5.
            },
            3.
        )
        .is_none()
    );
    assert!(
        b.hit_test(
            next,
            MapPoint {
                x: node.bounds.x + 5.,
                y: node.bounds.y + 5.
            },
            3.
        )
        .is_some()
    );
    let c = a
        .rebuild(
            next,
            MapPrefs {
                attributes: MapAttributes::KeysOnly,
                ..MapPrefs::default()
            },
            &[],
        )
        .unwrap();
    assert!(c.nodes.iter().all(|n| n.rows.iter().all(|r| r.attnum != 3)));
}
#[test]
fn composite_details_are_exact_and_oversized_details_refuse_before_allocation() {
    let mut s = snapshot();
    s.tables[0].name = "a\"b".into();
    s.tables[0].columns[1].name = "line\n雪".into();
    let a = scene(s);
    let details = a
        .details(
            a.edge_selection(EdgeIdentity {
                database_oid: 1,
                constraint_oid: 20,
            })
            .unwrap(),
        )
        .unwrap();
    assert!(details.contains("\"a\"\"b\""));
    assert!(details.contains("Pair 1: \"line\n雪\" (attnum 2) → \"c1\" (attnum 1)"));
    assert!(details.contains("ON UPDATE: CASCADE"));
    let mut s = snapshot();
    s.tables[0].columns[0].comment = Some("line\n".repeat(128));
    let a = scene(s);
    assert!(a.details(a.node_selection(id(10)).unwrap()).is_err());
    let mut s = snapshot();
    s.tables[0].columns = (1..=100)
        .map(|i| SchemaMapColumn {
            attnum: i,
            name: format!("c{i}"),
            data_type: "x".repeat(1000),
            nullable: false,
            primary_key: false,
            comment: None,
        })
        .collect();
    let a = scene(s);
    assert!(a.details(a.node_selection(id(10)).unwrap()).is_err());
}
#[test]
fn old_new_scene_and_export_leases_release_without_losing_old_capture() {
    let budget = Rc::new(Cell::new(0));
    let a = Scene::new(
        Arc::new(snapshot()),
        key(),
        MapPrefs::default(),
        &[],
        budget.clone(),
    )
    .unwrap();
    let old = budget.get();
    let b = a
        .rebuild(
            SceneKey {
                layout_revision: 4,
                ..key()
            },
            MapPrefs::default(),
            &[],
        )
        .unwrap();
    assert_eq!(budget.get(), old + b.retained_bytes());
    drop(b);
    assert_eq!(budget.get(), old);
    let other = Lease::new(budget.clone(), SHARED_BYTES - old - 1024).unwrap();
    assert!(a.rebuild(key(), MapPrefs::default(), &[]).is_err());
    assert!(a.accepts(a.node_selection(id(10)).unwrap()));
    drop(other);
    assert_eq!(budget.get(), old);
    let mut export = a
        .svg(Viewport {
            width: 800,
            height: 600,
            camera: Camera::default(),
        })
        .unwrap();
    assert!(budget.get() > old);
    let bytes = export.take_bytes().unwrap();
    assert!(export.take_bytes().is_err());
    assert!(budget.get() > old);
    drop(bytes);
    drop(export);
    assert_eq!(budget.get(), old);
    drop(a);
    assert_eq!(budget.get(), 0);
}
#[test]
fn camera_preserves_pointer_and_export_has_explicit_pixel_limit() {
    let camera = Camera {
        pan: MapPoint { x: 20., y: -10. },
        zoom: 0.5,
    };
    let pointer = MapPoint { x: 400., y: 300. };
    let world = camera.to_world(pointer).unwrap();
    let zoomed = camera.zoom_at(pointer, 2.).unwrap();
    assert_eq!(zoomed.to_world(pointer), Some(world));
    assert!(camera.zoom_at(pointer, f64::NAN).is_err());
    assert!(
        camera
            .pan_by(MapPoint {
                x: f64::INFINITY,
                y: 0.
            })
            .is_err()
    );
    let a = scene(snapshot());
    let fit = Camera::fit(a.bounds(), 800., 600.).unwrap();
    let r = a.bounds();
    for p in [
        MapPoint { x: r.x, y: r.y },
        MapPoint {
            x: r.x + r.width,
            y: r.y + r.height,
        },
    ] {
        let p = fit.to_screen(p).unwrap();
        assert!(p.x >= 0. && p.x <= 800. && p.y >= 0. && p.y <= 600.);
    }
    assert_eq!(
        Viewport {
            width: 800,
            height: 600,
            camera
        }
        .raster_size(2),
        Ok((1600, 1200))
    );
    assert!(
        Viewport {
            width: 2000,
            height: 1500,
            camera
        }
        .raster_size(2)
        .is_err()
    );
}
#[test]
fn positions_are_oid_bound_and_invalid_overlay_keeps_budget_unchanged() {
    let a = scene(snapshot());
    let positions = [
        SavedPosition {
            identity: id(10),
            position: MapPoint { x: 1000., y: 2000. },
        },
        SavedPosition {
            identity: id(999),
            position: MapPoint { x: 1., y: 1. },
        },
    ];
    let b = a.rebuild(key(), MapPrefs::default(), &positions).unwrap();
    let n = b.nodes.iter().find(|n| n.identity == id(10)).unwrap();
    assert_eq!((n.bounds.x, n.bounds.y), (1000., 2000.));
    let before = a.lease.budget.get();
    for bad in [MapPoint { x: f64::NAN, y: 0. }, MapPoint { x: 1e7, y: 0. }] {
        assert!(
            a.rebuild(
                key(),
                MapPrefs::default(),
                &[SavedPosition {
                    identity: id(10),
                    position: bad
                }]
            )
            .is_err()
        );
        assert_eq!(a.lease.budget.get(), before);
    }
    assert!(
        a.rebuild(key(), MapPrefs::default(), &[positions[0], positions[0]])
            .is_err()
    );
    assert!(
        a.rebuild(
            key(),
            MapPrefs::default(),
            &[SavedPosition {
                identity: SchemaMapIdentity {
                    database_oid: 2,
                    relation_oid: 10
                },
                position: MapPoint { x: 0., y: 0. }
            }]
        )
        .is_err()
    );
}
#[test]
fn svg_escapes_metadata_and_exports_current_camera_from_shared_paths() {
    let mut s = snapshot();
    s.tables[0].name = "<script>&\"雪".into();
    let a = scene(s);
    let v = Viewport {
        width: 800,
        height: 600,
        camera: Camera {
            pan: MapPoint { x: 12., y: 23. },
            zoom: 0.75,
        },
    };
    let export = a.svg(v).unwrap();
    let text = std::str::from_utf8(export.bytes().unwrap()).unwrap();
    assert!(!text.contains("<script>"));
    assert!(text.contains("&lt;script&gt;&amp;&quot;雪"));
    assert!(text.contains("translate(12 23) scale(0.75)"));
    assert!(text.contains("fill=\"white\""));
    assert!(text.contains("viewBox=\"0 0 800 600\""));
    let edge = &a.edges[0];
    if let Path::Curve(p) = &edge.path {
        assert!(text.contains(&format!(
            "M {} {} C {} {}, {} {}, {} {}",
            p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y, p[3].x, p[3].y
        )));
    }
    let off = a
        .svg(Viewport {
            camera: Camera {
                pan: MapPoint {
                    x: 100000.,
                    y: 100000.,
                },
                zoom: 1.,
            },
            ..v
        })
        .unwrap();
    assert!(
        !std::str::from_utf8(off.bytes().unwrap())
            .unwrap()
            .contains("data-relation-oid")
    );
}

#[test]
fn drag_updates_incident_paths_without_changing_capacity_and_refuses_atomically() {
    let mut a = Scene::new(
        Arc::new(snapshot()),
        key(),
        MapPrefs {
            routing: MapRouting::Step,
            ..MapPrefs::default()
        },
        &[],
        Rc::new(Cell::new(0)),
    )
    .unwrap();
    let before = a.actual_heap_bytes();
    let retained = a.retained_bytes();
    let old = a.node_selection(id(10)).unwrap();
    let moved = SceneKey {
        layout_revision: 4,
        ..key()
    };
    a.move_node(moved, id(10), MapPoint { x: 1300., y: 500. })
        .unwrap();
    assert_eq!(before, a.actual_heap_bytes());
    assert_eq!(retained, a.retained_bytes());
    assert!(!a.accepts(old));
    for edge in &a.edges {
        let fk = &a.snapshot.foreign_keys[edge.foreign_key_index];
        assert_eq!(
            edge.path.points()[0],
            a.anchor(fk.source, fk.columns[0].source, true).unwrap()
        );
        assert_eq!(
            *edge.path.points().last().unwrap(),
            a.anchor(fk.target, fk.columns[0].target, false).unwrap()
        );
    }
    let bounds = a.bounds();
    let paths = a.edges.iter().map(|e| e.path.clone()).collect::<Vec<_>>();
    assert!(
        a.move_node(
            SceneKey {
                layout_revision: 5,
                ..key()
            },
            id(10),
            MapPoint { x: 999999., y: 0. }
        )
        .is_err()
    );
    assert_eq!(a.key(), moved);
    assert_eq!(a.bounds(), bounds);
    assert_eq!(
        paths,
        a.edges.iter().map(|e| e.path.clone()).collect::<Vec<_>>()
    );
    assert!(
        a.move_node(key(), id(10), MapPoint { x: 0., y: 0. })
            .is_err()
    );
}

#[test]
fn detail_pages_preserve_every_character_across_line_and_byte_limits() {
    let mut s = snapshot();
    s.tables[0].columns = (1..=100)
        .map(|i| SchemaMapColumn {
            attnum: i,
            name: format!("c{i}"),
            data_type: "雪".repeat(2600),
            nullable: false,
            primary_key: false,
            comment: Some("one\r\ntwo\n".repeat(10)),
        })
        .collect();
    let a = scene(s);
    let selected = a.node_selection(id(10)).unwrap();
    assert!(a.details(selected).is_err());
    let mut full = String::new();
    a.write_details(selected, &mut full).unwrap();
    let mut assembled = String::new();
    let mut page = 0;
    loop {
        let result = a.details_page(selected, page).unwrap();
        assert_eq!(result.page, page);
        assert!(result.text.len() <= MAX_DETAIL_BYTES);
        assert!(
            result
                .text
                .bytes()
                .filter(|b| matches!(b, b'\r' | b'\n'))
                .count()
                < MAX_DETAIL_LINES
        );
        assembled.push_str(&result.text);
        if !result.next {
            break;
        }
        page += 1;
        assert!(page < 100);
    }
    assert!(page > 0);
    assert_eq!(assembled, full);
    assert!(a.details_page(selected, page + 1).is_err());
}

#[test]
fn generated_graph_svg_renders_to_two_times_png_with_owned_source_retained() {
    let graph = scene(snapshot());
    let viewport = Viewport {
        width: 800,
        height: 600,
        camera: Camera::fit(graph.bounds(), 800., 600.).unwrap(),
    };
    let mut svg = graph.svg(viewport).unwrap();
    let plan = crate::schema_map_png::PngPlan::new(&svg)
        .unwrap()
        .with_working_limit(crate::schema_map_png::MAX_WORKING_BYTES)
        .unwrap();
    let bytes = plan
        .encode(
            svg.take_bytes().unwrap(),
            &dbunk_lib::backend::result_files::Cancellation::default(),
        )
        .unwrap();
    let reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .unwrap();
    assert_eq!((reader.info().width, reader.info().height), (1600, 1200));
    assert_eq!(reader.info().color_type, png::ColorType::Rgba);
}

#[test]
fn composite_captions_get_adaptive_clear_rank_gaps_and_shared_svg_bounds() {
    let mut s = snapshot();
    s.tables.truncate(3);
    s.foreign_keys = vec![fk(20, 10, 11), fk(21, 11, 12), fk(22, 10, 12)];
    for table in &mut s.tables {
        table.columns[0].name = "tenant".into();
        table.columns[1].name = "parent_id".into();
    }
    let a = scene(s.clone());
    for edge in a.edges() {
        let rect = edge.label_bounds();
        assert!(rect.valid());
        assert!(rect.width > 140. && rect.width < 400.);
        assert!(a.nodes().iter().all(|n| !n.bounds.intersects(rect)));
        assert!(a.bounds().contains(MapPoint {
            x: rect.x,
            y: rect.y
        }));
        assert!(a.bounds().contains(MapPoint {
            x: rect.x + rect.width,
            y: rect.y + rect.height
        }));
        let source = &a.nodes()[edge.source];
        assert!(rect.x > source.bounds.x + source.bounds.width);
        assert!(
            a.hit_test(
                a.key(),
                MapPoint {
                    x: rect.x + 2.,
                    y: rect.y + 2.
                },
                0.
            )
            .is_some()
        );
    }
    for pair in a.nodes().windows(2) {
        let gap = pair[1].bounds.x - pair[0].bounds.x - NODE_WIDTH;
        assert!((140.0..400.0).contains(&gap));
    }
    s.tables.reverse();
    s.foreign_keys.reverse();
    let b = scene(s);
    assert_eq!(
        a.nodes()
            .iter()
            .map(|n| (n.identity, n.bounds))
            .collect::<Vec<_>>(),
        b.nodes()
            .iter()
            .map(|n| (n.identity, n.bounds))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        a.edges()
            .iter()
            .map(|e| (e.identity, e.label_bounds()))
            .collect::<Vec<_>>(),
        b.edges()
            .iter()
            .map(|e| (e.identity, e.label_bounds()))
            .collect::<Vec<_>>()
    );
    let svg = a
        .svg(Viewport {
            width: 2000,
            height: 1000,
            camera: Camera::default(),
        })
        .unwrap();
    let text = std::str::from_utf8(svg.bytes().unwrap()).unwrap();
    for edge in a.edges() {
        let r = edge.label_bounds();
        assert!(text.contains(&format!(
            "<clipPath id=\"edge-label-{}\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"/>",
            edge.identity.constraint_oid, r.x, r.y, r.width, r.height
        )));
    }
}

#[test]
fn dragging_an_unrelated_node_over_a_caption_repositions_it_without_allocating() {
    let mut a = scene(snapshot());
    let edge = a
        .edges()
        .iter()
        .find(|e| e.source != 3 && e.target != 3)
        .unwrap();
    let r = edge.label_bounds();
    let before = a.actual_heap_bytes();
    let key = SceneKey {
        layout_revision: 4,
        ..a.key()
    };
    a.move_node(key, id(13), MapPoint { x: r.x, y: r.y })
        .unwrap();
    assert_eq!(a.actual_heap_bytes(), before);
    for edge in a.edges() {
        assert!(
            a.nodes()
                .iter()
                .all(|node| !node.bounds.intersects(edge.label_bounds()))
        );
    }
    let saved = a
        .nodes()
        .iter()
        .map(|n| SavedPosition {
            identity: n.identity,
            position: MapPoint {
                x: n.bounds.x,
                y: n.bounds.y,
            },
        })
        .collect::<Vec<_>>();
    let rebuilt = a
        .rebuild(
            SceneKey {
                layout_revision: 5,
                ..key
            },
            a.prefs,
            &saved,
        )
        .unwrap();
    assert_eq!(
        a.edges()
            .iter()
            .map(|e| (e.identity, e.label_bounds()))
            .collect::<Vec<_>>(),
        rebuilt
            .edges()
            .iter()
            .map(|e| (e.identity, e.label_bounds()))
            .collect::<Vec<_>>()
    );
    for edge in rebuilt.edges() {
        assert!(
            rebuilt
                .nodes()
                .iter()
                .all(|node| !node.bounds.intersects(edge.label_bounds()))
        );
    }
}

use super::*;
use dbunk_lib::backend::{
    completion::{CompletionColumn, CompletionRelationKind},
    objects::{PgCatalogEntry, PgSchemaObjects},
};
fn handle() -> CompletionHandle {
    let h = CompletionHandle::new(Rc::new(Cell::new(0)), async_channel::bounded(1).0);
    h.bind(Some("connection".into()));
    h.set_connected(true);
    h
}
fn catalog() -> PgObjectCatalog {
    PgObjectCatalog {
        schemas: vec![PgSchemaObjects {
            name: "public".into(),
            tables: vec![PgCatalogEntry {
                name: "MixedCase".into(),
                identity_args: None,
                comment: None,
                type_class: None,
            }],
            views: vec![],
            materialized_views: vec![],
            foreign_tables: vec![],
            sequences: vec![],
            functions: vec![],
            procedures: vec![],
            aggregates: vec![],
            types: vec![],
            domains: vec![],
            extensions: vec![],
        }],
        event_triggers: vec![],
        roles: vec![],
        tablespaces: vec![],
        truncated: vec![],
    }
}
fn columns() -> CompletionColumns {
    CompletionColumns {
        schema: "public".into(),
        relation: "MixedCase".into(),
        relation_oid: 42,
        kind: CompletionRelationKind::Table,
        columns: vec![CompletionColumn {
            name: "id".into(),
            data_type: "integer".into(),
            ordinal_position: 1,
            is_primary_key: true,
        }],
    }
}
#[test]
fn stale_reply_never_settles_new_binding() {
    let h = handle();
    h.refresh();
    let old = h.pending_request().unwrap().id();
    h.bind(Some("different".into()));
    h.set_connected(true);
    h.refresh();
    let new = h.pending_request().unwrap().id();
    assert!(new > old);
    h.accept_catalog(old, Ok(catalog()));
    assert_eq!(h.pending_request().unwrap().id(), new);
    assert!(h.0.borrow().cache.is_none());
    h.accept_catalog(new, Ok(catalog()));
    assert!(h.0.borrow().cache.is_some());
}
#[test]
fn editor_move_allows_cache_but_prevents_menu_refresh() {
    let h = handle();
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.editor_changed();
    h.accept_catalog(id, Ok(catalog()));
    assert!(!h.take_refresh());
    assert!(h.0.borrow().cache.is_some());
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.accept_catalog(id, Ok(catalog()));
    assert!(h.take_refresh());
    assert!(!h.take_refresh());
}
#[test]
fn refusal_preserves_last_good_capture_and_budget() {
    let h = handle();
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.accept_catalog(id, Ok(catalog()));
    assert_eq!(h.0.borrow().budget.get(), CACHE_BYTES);
    let mut bad = catalog();
    bad.schemas[0].name = String::with_capacity(CACHE_TEXT_BYTES + 1);
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.accept_catalog(id, Ok(bad));
    assert_eq!(
        h.0.borrow().cache.as_ref().unwrap().schemas[0].name,
        "public"
    );
    assert_eq!(h.0.borrow().budget.get(), CACHE_BYTES);
    assert!(h.status().contains("unavailable"));
    h.set_connected(false);
    assert_eq!(h.0.borrow().budget.get(), 0);
}
#[test]
fn request_coalescing_exact_columns_and_explicit_retry() {
    let h = handle();
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.mark_dispatched(id);
    assert!(h.pending_request().is_none());
    h.accept_catalog(id, Ok(catalog()));
    h.0.borrow_mut()
        .queue(Some(("public".into(), "MixedCase".into())));
    let id = h.pending_request().unwrap().id();
    let mut wrong = columns();
    wrong.relation = "wrong".into();
    h.accept_columns(id, Ok(wrong));
    assert!(h.0.borrow().cache.as_ref().unwrap().columns.is_none());
    h.0.borrow_mut()
        .queue(Some(("public".into(), "MixedCase".into())));
    assert!(h.pending_request().is_none());
    h.refresh();
    assert!(matches!(
        h.pending_request(),
        Some(MetadataRequest::Catalog { .. })
    ));
}
#[test]
fn column_capacity_and_order_are_bounded_before_retention() {
    let mut c = columns();
    assert!(validate_columns(&c).is_ok());
    c.columns[0].data_type = String::with_capacity(CACHE_TEXT_BYTES + 1);
    assert!(validate_columns(&c).is_err());
    let mut c = columns();
    c.columns.push(c.columns[0].clone());
    assert!(validate_columns(&c).is_err());
}
#[test]
fn composing_suppresses_new_reads_and_refresh_without_cancelling_owner() {
    let h = handle();
    h.refresh();
    let id = h.pending_request().unwrap().id();
    h.set_composing(true);
    h.refresh();
    assert_eq!(h.pending_request().unwrap().id(), id);
    h.accept_catalog(id, Ok(catalog()));
    assert!(!h.take_refresh());
}

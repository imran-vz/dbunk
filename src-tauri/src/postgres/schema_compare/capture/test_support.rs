//! Synthetic, complete-inventory fixtures for the pure diff tests. This module
//! is unavailable to production callers; real captures come only from the reader.
use super::{
    data::{WireField, WireValue},
    *,
};

pub(crate) struct TestRelation {
    pub oid: u32,
    pub entry: InventoryEntry,
    pub fields: Vec<(FieldPath, CapturedValue)>,
}

pub(crate) fn fixture(
    budget: &Budget,
    connection: &str,
    schema: &str,
    version: u32,
    rows: Vec<TestRelation>,
) -> CapturedEndpoint {
    let mut capture = CapturedEndpoint::new(
        CaptureMetadata {
            endpoint: Endpoint {
                connection_id: connection.into(),
                schema: schema.into(),
            },
            server_version: version.to_string(),
            server_version_num: version,
            captured_at: "2026-09-06T00:00:00Z".into(),
        },
        budget,
    )
    .unwrap();
    for row in rows {
        capture.oids.push(row.oid);
        capture.inventory.push(row.entry);
        for (path, value) in row.fields {
            capture
                .push(WireField {
                    table_oid: row.oid,
                    path,
                    fact: WireValue::Plain(value),
                })
                .unwrap();
        }
    }
    capture.certify(|| Ok(())).unwrap();
    capture
}

pub(crate) fn shuffled(capture: &mut CapturedEndpoint) {
    capture.inventory.reverse();
    capture.oids.reverse();
    capture.fields.reverse();
}

pub(crate) fn share_snapshot(source: &CapturedEndpoint, target: &mut CapturedEndpoint) {
    target.snapshot = source.snapshot.clone();
}

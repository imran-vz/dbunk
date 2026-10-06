//! Plan 032 §3.1 and §3.2: the one path that pushes staged-change state into
//! the grid, and the per-tab write policy taken from the tab's own connection.
use super::TableView;
use crate::{
    data_model::TablePolicy,
    grid::{InlineEditorSlot, TableEditing},
};
use dbunk_lib::backend::DevelopmentConnection;
use gpui::{Context, SharedString};

/// The policy for the connection `id` among `records`. A missing record, a
/// non-PostgreSQL record or an unbound tab gets the strictest `UNKNOWN`
/// policy; the backend stays authoritative either way (ADR-0024).
pub(super) fn policy_for(records: &[DevelopmentConnection], id: Option<&str>) -> TablePolicy {
    id.and_then(|id| records.iter().find(|record| record.id == id))
        .filter(|record| record.postgres.is_some())
        .map_or(TablePolicy::UNKNOWN, TablePolicy::from_connection)
}

/// What the grid may offer. `live` is false while disconnected, loading or
/// otherwise unable to accept a staged edit; checkboxes stay visible then so
/// the layout does not jump, but are hidden on read-only connections.
pub(super) fn table_editing(
    live: bool,
    read_only: bool,
    can_edit: Result<(), SharedString>,
) -> TableEditing {
    let reason = if !live {
        Some(SharedString::from("Wait for the table to finish loading"))
    } else {
        can_edit.err()
    };
    TableEditing {
        editable: reason.is_none(),
        checkboxes: !read_only,
        reason,
    }
}

impl TableView {
    pub fn set_connection_metadata(
        &mut self,
        records: &[DevelopmentConnection],
        cx: &mut Context<Self>,
    ) {
        let policy = policy_for(records, self.connection.as_deref());
        if self.changes.read(cx).policy() != policy {
            self.changes
                .update(cx, |changes, cx| changes.set_policy(policy, cx));
        }
        self.sync_grid(cx);
        cx.notify();
    }

    /// Cheap and idempotent: the grid ignores an unchanged overlay `Rc`,
    /// editor slot, editing state and sort.
    pub(super) fn sync_grid(&mut self, cx: &mut Context<Self>) {
        let (overlay, slot, can_edit, policy) = {
            let changes = self.changes.read(cx);
            (
                changes.overlay(),
                changes.inline_editor(),
                changes.can_edit_now(),
                changes.policy(),
            )
        };
        let slot = slot.map(|(cell, source, view)| InlineEditorSlot { cell, source, view });
        let live = self.editable && !self.busy && self.controls.is_some();
        let editing = table_editing(live, policy.read_only, can_edit);
        let sort = self.state.sort.clone();
        self.grid.update(cx, |grid, cx| {
            grid.set_overlay(overlay, cx);
            grid.set_inline_editor(slot, cx);
            grid.set_table_editing(editing, cx);
            grid.set_sort_indicators(sort, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_model::EffectiveSafeMode;
    use dbunk_lib::backend::{
        DevelopmentConnectionOrganization, DevelopmentEnvironment, DevelopmentPostgresConnection,
        DevelopmentSafeMode,
    };

    fn postgres(
        id: &str,
        environment: DevelopmentEnvironment,
        read_only: bool,
    ) -> DevelopmentConnection {
        let mut record = other(id, "postgres");
        record.environment = environment;
        record.postgres = Some(DevelopmentPostgresConnection {
            name: id.into(),
            host: "localhost".into(),
            port: 5432,
            database: "app".into(),
            user: "app".into(),
            environment,
            safe_mode: DevelopmentSafeMode::Inherit,
            read_only,
            tls: Default::default(),
            driver_options: Default::default(),
            ssh_tunnel: None,
        });
        record
    }

    fn other(id: &str, engine: &str) -> DevelopmentConnection {
        DevelopmentConnection {
            id: id.into(),
            name: id.into(),
            engine: engine.into(),
            organization: DevelopmentConnectionOrganization::default(),
            unsupported_reason: None,
            postgres: None,
            environment: DevelopmentEnvironment::Development,
            settings: None,
            last_activity_at: None,
        }
    }

    #[test]
    fn policy_comes_from_the_tabs_own_connection() {
        let records = [
            postgres("dev", DevelopmentEnvironment::Development, false),
            postgres("prod", DevelopmentEnvironment::Production, false),
        ];
        let policy = policy_for(&records, Some("prod"));
        assert_eq!(policy, TablePolicy::from_connection(&records[1]));
        assert_eq!(policy.environment, Some(DevelopmentEnvironment::Production));
        assert_eq!(policy.safe_mode, EffectiveSafeMode::Strict);
        assert_eq!(
            policy_for(&records, Some("dev")),
            TablePolicy::from_connection(&records[0])
        );
    }

    #[test]
    fn read_only_record_yields_a_read_only_policy() {
        let records = [postgres("ro", DevelopmentEnvironment::Staging, true)];
        assert!(policy_for(&records, Some("ro")).read_only);
    }

    #[test]
    fn missing_unbound_or_non_postgres_connections_are_unknown() {
        let records = [
            postgres("dev", DevelopmentEnvironment::Development, false),
            other("lite", "sqlite"),
        ];
        assert_eq!(policy_for(&records, Some("gone")), TablePolicy::UNKNOWN);
        assert_eq!(policy_for(&records, None), TablePolicy::UNKNOWN);
        assert_eq!(policy_for(&records, Some("lite")), TablePolicy::UNKNOWN);
        assert_eq!(policy_for(&[], Some("dev")), TablePolicy::UNKNOWN);
    }

    #[test]
    fn grid_editing_follows_liveness_and_policy() {
        let editing = table_editing(true, false, Ok(()));
        assert!(editing.editable && editing.checkboxes && editing.reason.is_none());
        let loading = table_editing(false, false, Ok(()));
        assert!(!loading.editable && loading.checkboxes && loading.reason.is_some());
        let read_only = table_editing(true, true, Err("Read-only connection".into()));
        assert!(!read_only.editable && !read_only.checkboxes);
        assert_eq!(read_only.reason.as_deref(), Some("Read-only connection"));
    }
}

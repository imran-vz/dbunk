//! Durable description only. A restored record cannot mint executable authority.
use super::WorkspaceError;
use crate::backend::maintenance::{
    MaintenanceIntent, MaintenancePreview, MaintenanceRelationKind, MaintenanceReview,
    MaintenanceSemantics, MaintenanceTarget,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceMaintenanceAction {
    Vacuum,
    Analyze,
    ReindexTable,
    Refresh,
    RefreshConcurrently,
}
impl From<MaintenanceIntent> for WorkspaceMaintenanceAction {
    fn from(intent: MaintenanceIntent) -> Self {
        match intent {
            MaintenanceIntent::Vacuum => Self::Vacuum,
            MaintenanceIntent::Analyze => Self::Analyze,
            MaintenanceIntent::ReindexTable => Self::ReindexTable,
            MaintenanceIntent::RefreshMaterializedView {
                concurrently: false,
            } => Self::Refresh,
            MaintenanceIntent::RefreshMaterializedView { concurrently: true } => {
                Self::RefreshConcurrently
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceMaintenanceKind {
    Table,
    PartitionedTable,
    MaterializedView,
}
impl From<MaintenanceRelationKind> for WorkspaceMaintenanceKind {
    fn from(kind: MaintenanceRelationKind) -> Self {
        match kind {
            MaintenanceRelationKind::Table => Self::Table,
            MaintenanceRelationKind::PartitionedTable => Self::PartitionedTable,
            MaintenanceRelationKind::MaterializedView => Self::MaterializedView,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceMaintenanceState {
    Staged,
    OutcomeUnknown,
    EffectsPossible,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceMaintenance {
    pub attempt_id: String,
    pub action: WorkspaceMaintenanceAction,
    pub database_oid: u32,
    pub database: String,
    pub namespace_oid: u32,
    pub schema: String,
    pub relation_oid: u32,
    pub name: String,
    pub kind: WorkspaceMaintenanceKind,
    pub sql: String,
    pub potentially_partial: bool,
    pub operation_timeout_ms: u32,
    pub statement_timeout_ms: Option<u32>,
    pub state: WorkspaceMaintenanceState,
}
impl WorkspaceMaintenance {
    pub fn from_review(review: &MaintenanceReview) -> Self {
        let target = review.target();
        let preview = review.preview();
        Self {
            attempt_id: review.attempt_id().into(),
            action: review.intent().into(),
            database_oid: target.database_oid(),
            database: target.database().into(),
            namespace_oid: target.namespace_oid(),
            schema: target.schema().into(),
            relation_oid: target.relation_oid(),
            name: target.name().into(),
            kind: target.kind().into(),
            sql: preview.sql.clone(),
            potentially_partial: preview.semantics == MaintenanceSemantics::PotentiallyPartial,
            operation_timeout_ms: preview.operation_timeout_ms,
            statement_timeout_ms: preview.statement_timeout_ms,
            state: WorkspaceMaintenanceState::Staged,
        }
    }
    pub fn matches(
        &self,
        attempt: &str,
        intent: MaintenanceIntent,
        target: &MaintenanceTarget,
        preview: &MaintenancePreview,
    ) -> bool {
        self.attempt_id == attempt
            && self.action == intent.into()
            && self.database_oid == target.database_oid()
            && self.database == target.database()
            && self.namespace_oid == target.namespace_oid()
            && self.schema == target.schema()
            && self.relation_oid == target.relation_oid()
            && self.name == target.name()
            && self.kind == target.kind().into()
            && self.sql == preview.sql
            && self.potentially_partial
                == (preview.semantics == MaintenanceSemantics::PotentiallyPartial)
            && self.operation_timeout_ms == preview.operation_timeout_ms
            && self.statement_timeout_ms == preview.statement_timeout_ms
    }
    pub fn validate(&self) -> Result<(), WorkspaceError> {
        let uuid = uuid::Uuid::parse_str(&self.attempt_id).ok();
        let name = |s: &str| !s.is_empty() && s.len() <= 63 && !s.contains('\0');
        let (command, partial) = match self.action {
            WorkspaceMaintenanceAction::Vacuum => ("VACUUM", true),
            WorkspaceMaintenanceAction::Analyze => ("ANALYZE", true),
            WorkspaceMaintenanceAction::ReindexTable => (
                "REINDEX TABLE",
                self.kind == WorkspaceMaintenanceKind::PartitionedTable,
            ),
            WorkspaceMaintenanceAction::Refresh
            | WorkspaceMaintenanceAction::RefreshConcurrently => {
                if self.kind != WorkspaceMaintenanceKind::MaterializedView {
                    return Err(WorkspaceError::InvalidSnapshot);
                }
                (
                    if self.action == WorkspaceMaintenanceAction::Refresh {
                        "REFRESH MATERIALIZED VIEW"
                    } else {
                        "REFRESH MATERIALIZED VIEW CONCURRENTLY"
                    },
                    false,
                )
            }
        };
        if !uuid.is_some_and(|id| id.get_version_num() == 4 && id.to_string() == self.attempt_id)
            || self.database_oid == 0
            || self.namespace_oid == 0
            || self.relation_oid == 0
            || ![&self.database, &self.schema, &self.name]
                .into_iter()
                .all(|s| name(s))
            || self.sql.len() > 1024
            || self.potentially_partial != partial
            || self.operation_timeout_ms
                != crate::backend::maintenance::MAINTENANCE_OPERATION_TIMEOUT_MS
        {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        let expected = format!(
            "{command} \"{}\".\"{}\"",
            self.schema.replace('"', "\"\""),
            self.name.replace('"', "\"\"")
        );
        if self.sql != expected {
            return Err(WorkspaceError::InvalidSnapshot);
        }
        Ok(())
    }
}

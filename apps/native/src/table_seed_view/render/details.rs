use super::*;
impl TableSeedView {
    pub(in super::super) fn job_label(&self, id: TableSeedAttemptId, cx: &gpui::App) -> String {
        let store = self.store.read(cx);
        let record = store
            .journal()
            .iter()
            .find(|record| record.attempt_id == id);
        let observation = store.jobs().iter().find(|job| job.attempt_id == id);
        let endpoint = record
            .map(|record| &record.description.endpoint)
            .or_else(|| observation.map(|job| &job.endpoint));
        let state = observation.map_or_else(
            || {
                record.map_or_else(
                    || "Unavailable".into(),
                    |record| format!("Recovered {:?}", record.state),
                )
            },
            |job| format!("{:?}: {:?}", job.phase, job.outcome),
        );
        endpoint.map_or_else(
            || format!("{id}: unavailable"),
            |endpoint| format!("{state} | {}.{}", endpoint.schema, endpoint.table),
        )
    }
    pub(in super::super) fn details(&self, cx: &gpui::App) -> Option<String> {
        let id = self.selected?;
        let record = self.journal(cx);
        let observation = self.observation(cx);
        let mut text = format!("Attempt {id}");
        if let Some(record) = record {
            text.push_str(&format!(
                "\nDurable recovery: {:?}\n{}",
                record.state,
                description(&record.description)
            ));
            if let Some(failure) = record.failure {
                text.push_str(&format!("\nRecorded failure: {failure}"));
            }
            if let Some(diagnostic) = &record.diagnostic {
                text.push_str(&diagnostic_text(diagnostic));
            }
        }
        if let Some(job) = observation {
            text.push_str(&format!("\nObserved phase: {:?}\nTransaction outcome: {:?}\nCleanup: {:?}\nRows generated: {} / {}",job.phase,job.outcome,job.cleanup,job.rows_generated,job.row_count));
            if let Some(issue) = job.issue {
                text.push_str(&format!("\nRecipe requires attention: {issue}"));
            }
            if let Some(failure) = job.failure {
                text.push_str(&format!("\nFailure: {failure}"));
            }
            if let Some(diagnostic) = &job.diagnostic {
                text.push_str(&diagnostic_text(diagnostic));
            }
            if record.is_none() {
                text.push_str(&format!(
                    "\nDestination: {} / {}.{}",
                    job.endpoint.connection_id, job.endpoint.schema, job.endpoint.table
                ));
            }
        } else {
            text.push_str("\nNo current backend attempt. Recovery cannot execute or prove rollback. Inspect the destination before reconciliation.");
        }
        Some(text)
    }
    pub(in super::super) fn review_column_text(&self, cx: &gpui::App) -> Option<String> {
        let review = self
            .store
            .read(cx)
            .review_payload()
            .filter(|review| Some(review.attempt_id()) == self.selected)?;
        let column = review.columns().get(self.review_column?)?;
        let spec = review
            .specs()
            .iter()
            .find(|spec| spec.column == column.name);
        let mut text = format!(
            "Exact column: {}\nType: {}\nResolved action: {}\nNullable: {}; default: {}; generated: {}; identity: {}",
            column.name,
            column.data_type,
            column_action(&column.action),
            column.nullable,
            column.has_default,
            column.generated,
            column.identity
        );
        if let Some(spec) = spec {
            match &spec.source {
                TableSeedSource::Auto { generator } => text.push_str(&format!(
                    "\nGenerator: {}",
                    generator.map_or("Auto", TableSeedGenerator::id)
                )),
                TableSeedSource::Default => text.push_str("\nUse database DEFAULT"),
                TableSeedSource::Constant { value } => {
                    text.push_str("\nExact constant (text):\n");
                    text.push_str(value);
                }
                TableSeedSource::Values { values } => {
                    text.push_str("\nExact value list (one entry per numbered line):");
                    for (index, value) in values.iter().enumerate() {
                        text.push_str(&format!("\n{}. ", index + 1));
                        text.push_str(value);
                    }
                }
            }
            if let Some(rate) = spec.null_rate {
                text.push_str(&format!("\nNULL percentage: {}", rate * 100.));
            }
        } else {
            text.push_str("\nAuto recipe with backend default NULL rate");
        }
        Some(text)
    }
}
pub(super) fn column_action(action: &TableSeedColumnAction) -> String {
    match action {
        TableSeedColumnAction::Default => "Database DEFAULT".into(),
        TableSeedColumnAction::Constant => "Exact constant".into(),
        TableSeedColumnAction::Values => "Value list".into(),
        TableSeedColumnAction::ForeignKey { schema, table } => {
            format!("Referenced tuples from {schema}.{table}")
        }
        TableSeedColumnAction::Auto => "Auto generator".into(),
        TableSeedColumnAction::UnsupportedNull => "Always NULL".into(),
    }
}
pub(super) fn description(value: &TableSeedDescription) -> String {
    let target = &value.connection;
    format!(
        "Destination: {} ({}) / {}.{}\nEndpoint: {}:{} / {} / user {} / environment {}\nSafe mode: {}; read-only: {}\nObserved database OID: {}; relation OID: {}\nRows: {}; exact seed: {}; frozen clock (Unix seconds): {}\nColumns: {} inserted; {} defaulted\nRecipe identity: {}\nRecorded recipe summary (limited previews; full literals are session-only):\n{}\nSummary clipped: {}",
        target.connection_name,
        value.endpoint.connection_id,
        value.endpoint.schema,
        value.endpoint.table,
        target.host,
        target.port,
        target.database,
        target.user,
        target.environment,
        target.safe_mode,
        target.read_only,
        value.database_oid,
        value.relation_oid,
        value.row_count,
        value.seed_used,
        value.clock_epoch_seconds,
        value.inserted_columns,
        value.defaulted_columns,
        value.recipe_sha256,
        value.recipe_summary,
        value.recipe_summary_truncated
    )
}
fn diagnostic_text(value: &TableSeedDiagnostic) -> String {
    let mut text = String::new();
    for (label, value) in [
        ("SQLSTATE", &value.sqlstate),
        ("Constraint", &value.constraint),
        ("Column", &value.column),
        ("Parent schema", &value.parent_schema),
        ("Parent table", &value.parent_table),
    ] {
        if let Some(value) = value {
            text.push_str(&format!("\n{label}: {value}"));
        }
    }
    text
}

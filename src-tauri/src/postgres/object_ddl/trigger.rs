//! Trigger operations. Triggers are addressed by table and name, not by an
//! Object Ref (ADR-0027).

use serde::{Deserialize, Serialize};

use super::common::*;
use super::fragment::{validate_fragment, FragmentContext};
use super::{ObjectOperation, PgObjectError, RenderedOp};
use crate::{quote_double, quote_literal};

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTriggerTiming {
    Before,
    After,
    InsteadOf,
}

impl PgTriggerTiming {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Before => "BEFORE",
            Self::After => "AFTER",
            Self::InsteadOf => "INSTEAD OF",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum PgTriggerEvent {
    Insert,
    Update { columns: Vec<String> },
    Delete,
    Truncate,
}

impl PgTriggerEvent {
    pub(super) fn discriminant(&self) -> u8 {
        match self {
            Self::Insert => 0,
            Self::Update { .. } => 1,
            Self::Delete => 2,
            Self::Truncate => 3,
        }
    }

    pub(super) fn label(&self) -> &'static str {
        match self {
            Self::Insert => "INSERT",
            Self::Update { .. } => "UPDATE",
            Self::Delete => "DELETE",
            Self::Truncate => "TRUNCATE",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTriggerLevel {
    Row,
    Statement,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PgTriggerMode {
    Enable,
    Disable,
    EnableReplica,
    EnableAlways,
}

impl PgTriggerMode {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Enable => "ENABLE",
            Self::Disable => "DISABLE",
            Self::EnableReplica => "ENABLE REPLICA",
            Self::EnableAlways => "ENABLE ALWAYS",
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTriggerOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub timing: PgTriggerTiming,
    pub events: Vec<PgTriggerEvent>,
    pub for_each: PgTriggerLevel,
    pub when: Option<String>,
    pub function_schema: String,
    pub function_name: String,
    pub arguments: Vec<String>,
    pub or_replace: bool,
}

impl ObjectOperation for CreateTriggerOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            timing,
            events,
            for_each,
            when,
            function_schema,
            function_name,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "trigger", name)?;
            require_name(op_index, "trigger function schema", function_schema)?;
            require_name(op_index, "trigger function", function_name)?;
            if events.is_empty() {
                return invalid(op_index, "trigger events cannot be empty");
            }
            let mut seen = Vec::with_capacity(events.len());
            for event in events {
                if seen.contains(&event.discriminant()) {
                    return invalid(
                        op_index,
                        format!("trigger event {} is listed more than once", event.label()),
                    );
                }
                seen.push(event.discriminant());
                if let PgTriggerEvent::Update { columns } = event {
                    for column in columns {
                        require_name(op_index, "trigger update column", column)?;
                    }
                }
                if matches!(event, PgTriggerEvent::Truncate)
                    && *for_each != PgTriggerLevel::Statement
                {
                    return invalid(op_index, "TRUNCATE triggers must be FOR EACH STATEMENT");
                }
            }
            if *timing == PgTriggerTiming::InsteadOf {
                if *for_each != PgTriggerLevel::Row {
                    return invalid(op_index, "INSTEAD OF triggers must be FOR EACH ROW");
                }
                if when.is_some() {
                    return invalid(op_index, "INSTEAD OF triggers cannot have a WHEN condition");
                }
                if events
                    .iter()
                    .any(|event| matches!(event, PgTriggerEvent::Truncate))
                {
                    return invalid(op_index, "INSTEAD OF triggers cannot fire on TRUNCATE");
                }
            }
            if let Some(when) = when {
                validate_fragment(
                    op_index,
                    "trigger WHEN condition",
                    when,
                    FragmentContext::Embedded,
                )?;
            }
            Ok(())
        }
    }

    fn fragments(&self) -> Vec<&str> {
        self.when.iter().map(String::as_str).collect()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            timing,
            events,
            for_each,
            when,
            function_schema,
            function_name,
            arguments,
            or_replace,
        } = self;
        let rendered = {
            let events_sql = events
                .iter()
                .map(|event| match event {
                    PgTriggerEvent::Update { columns } if !columns.is_empty() => {
                        format!("UPDATE OF {}", render_ident_list(columns))
                    }
                    other => other.label().to_string(),
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            let when_sql = when
                .as_ref()
                .map(|when| format!(" WHEN ({when})"))
                .unwrap_or_default();
            let arguments_sql = arguments
                .iter()
                .map(|argument| quote_literal(argument))
                .collect::<Vec<_>>()
                .join(", ");
            RenderedOp {
            sql: format!(
                "CREATE {}TRIGGER {} {} {events_sql} ON {} FOR EACH {}{when_sql} EXECUTE FUNCTION {}({arguments_sql});",
                if *or_replace { "OR REPLACE " } else { "" },
                quote_double(name),
                timing.sql(),
                qualified(schema, table),
                match for_each {
                    PgTriggerLevel::Row => "ROW",
                    PgTriggerLevel::Statement => "STATEMENT",
                },
                qualified(function_schema, function_name)
            ),
            summary: format!(
                "{} trigger {name} on {schema}.{table}",
                if *or_replace { "Create or replace" } else { "Create" }
            ),
            destructive: *or_replace,
            transactional: true,
        }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropTriggerOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub cascade: bool,
}

impl ObjectOperation for DropTriggerOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "trigger", name)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            cascade,
        } = self;
        let rendered = {
            let behavior = if *cascade { "CASCADE" } else { "RESTRICT" };
            RenderedOp {
                sql: format!(
                    "DROP TRIGGER {} ON {} {behavior};",
                    quote_double(name),
                    qualified(schema, table)
                ),
                summary: format!("Drop trigger {name} on {schema}.{table} ({behavior})"),
                destructive: true,
                transactional: true,
            }
        };
        Ok(rendered)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetTriggerEnabledOp {
    pub schema: String,
    pub table: String,
    pub name: String,
    pub mode: PgTriggerMode,
}

impl ObjectOperation for SetTriggerEnabledOp {
    fn validate(&self, op_index: usize) -> Result<(), PgObjectError> {
        let Self {
            schema,
            table,
            name,
            ..
        } = self;
        {
            require_table(op_index, schema, table)?;
            require_name(op_index, "trigger", name)
        }
    }

    fn fragments(&self) -> Vec<&str> {
        Vec::new()
    }

    fn render(&self, _op_index: usize) -> Result<RenderedOp, PgObjectError> {
        let Self {
            schema,
            table,
            name,
            mode,
        } = self;
        let rendered = RenderedOp {
            sql: format!(
                "ALTER TABLE {} {} TRIGGER {};",
                qualified(schema, table),
                mode.sql(),
                quote_double(name)
            ),
            summary: format!(
                "{} trigger {name} on {schema}.{table}",
                match mode {
                    PgTriggerMode::Enable => "Enable",
                    PgTriggerMode::Disable => "Disable",
                    PgTriggerMode::EnableReplica => "Enable (replica)",
                    PgTriggerMode::EnableAlways => "Enable (always)",
                }
            ),
            // A disabled trigger silently stops enforcing whatever it
            // guarded; the classifier sees only a harmless ALTER.
            destructive: *mode == PgTriggerMode::Disable,
            transactional: true,
        };
        Ok(rendered)
    }
}

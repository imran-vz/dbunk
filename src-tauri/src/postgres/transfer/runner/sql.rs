use super::super::csv::{self, CsvOptions};
use super::{Review, EXPORT_FIELD_LIMIT_SENTINEL, EXPORT_RECORD_LIMIT_SENTINEL};

pub(super) fn import(review: &Review, mapping: &[(usize, String)]) -> String {
    let columns = mapping
        .iter()
        .map(|(_, target)| crate::quote_double(target))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "COPY {} ({columns}) FROM STDIN WITH ({})",
        qualified(review),
        csv_options(&review.payload.options, false)
    )
}

pub(super) fn export(review: &Review) -> String {
    export_columns(
        &review.payload.schema,
        &review.payload.table,
        &review
            .relation
            .columns
            .iter()
            .map(|c| c.public.name.as_str())
            .collect::<Vec<_>>(),
        &review.payload.options,
    )
}

/// Shared bounded PostgreSQL text projection used by native table copy.
pub(crate) fn export_columns(
    schema: &str,
    table: &str,
    names: &[&str],
    options: &CsvOptions,
) -> String {
    const SOURCE_ALIAS: &str = "__dbunk_source";
    const OUTPUT_ALIAS: &str = "__dbunk_output";
    let columns = names
        .iter()
        .map(|column| {
            format!(
                "{}.{}",
                crate::quote_double(OUTPUT_ALIAS),
                crate::quote_double(column)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let canonical_columns = names.iter()
        .map(|column| {
            let value = format!(
                "{}.{}",
                crate::quote_double(SOURCE_ALIAS),
                crate::quote_double(column)
            );
            format!(
                "CASE WHEN {value} IS NOT DISTINCT FROM NULL THEN NULL::text ELSE pg_catalog.format('%s', {value}) END AS {}",
                crate::quote_double(column)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let canonical_select = if canonical_columns.is_empty() {
        format!(
            "SELECT FROM {} AS {} OFFSET 0",
            qualified_names(schema, table),
            crate::quote_double(SOURCE_ALIAS)
        )
    } else {
        format!(
            "SELECT {canonical_columns} FROM {} AS {} OFFSET 0",
            qualified_names(schema, table),
            crate::quote_double(SOURCE_ALIAS)
        )
    };
    let source = format!(
        "({canonical_select}) AS {} WHERE {}",
        crate::quote_double(OUTPUT_ALIAS),
        export_size_guard(names, options, OUTPUT_ALIAS)
    );
    let select = if columns.is_empty() {
        format!("SELECT FROM {source}")
    } else {
        format!("SELECT {columns} FROM {source}")
    };
    format!(
        "COPY ({select}) TO STDOUT WITH ({})",
        csv_options(options, options.header)
    )
}

/// PostgreSQL emits a complete CSV row as one protocol CopyData message. The
/// source subquery computes each canonical type output once. Its `OFFSET 0`
/// barrier preserves streaming while preventing the guard and COPY projection
/// from separately invoking type output. The row estimate is deliberately
/// conservative: every non-null byte may need doubling plus surrounding quotes.
fn export_size_guard(names: &[&str], options: &CsvOptions, alias: &str) -> String {
    if names.is_empty() {
        return "TRUE".into();
    }
    let values = names
        .iter()
        .map(|column| {
            format!(
                "{}.{}",
                crate::quote_double(alias),
                crate::quote_double(column)
            )
        })
        .collect::<Vec<_>>();
    let fields = values
        .iter()
        .map(|value| {
            format!(
                "({value} IS NOT DISTINCT FROM NULL OR {} <= {})",
                utf8_length(value),
                csv::MAX_FIELD_BYTES
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let null_bytes = options.null_token.len();
    let row_bytes = values
        .iter()
        .map(|value| {
            format!(
                "CASE WHEN {value} IS NOT DISTINCT FROM NULL THEN {null_bytes}::bigint ELSE (2::bigint * {} + 2) END",
                utf8_length(value)
            )
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let separators = values.len(); // delimiters plus the line ending
    format!(
        "CASE WHEN NOT ({fields}) THEN {} WHEN ({row_bytes} + {separators}) > {} THEN {} ELSE TRUE END",
        limit_failure(EXPORT_FIELD_LIMIT_SENTINEL),
        csv::MAX_RECORD_BYTES,
        limit_failure(EXPORT_RECORD_LIMIT_SENTINEL)
    )
}

fn utf8_length(value: &str) -> String {
    format!("pg_catalog.octet_length(pg_catalog.convert_to({value}, 'UTF8'))")
}

fn limit_failure(sentinel: &str) -> String {
    let prefix = sentinel
        .strip_suffix('0')
        .expect("export limit sentinels end in the runtime zero");
    format!(
        "(({} || (pg_catalog.pg_backend_pid() * 0)::text)::integer = 0)",
        crate::quote_literal(prefix)
    )
}

fn csv_options(options: &CsvOptions, header: bool) -> String {
    format!(
        "FORMAT csv, DELIMITER {}, QUOTE {}, ESCAPE {}, NULL {}, HEADER {}, ENCODING {}",
        crate::quote_literal(&options.delimiter),
        crate::quote_literal(&options.quote),
        crate::quote_literal(&options.escape),
        crate::quote_literal(&options.null_token),
        if header { "true" } else { "false" },
        crate::quote_literal("UTF8")
    )
}

pub(super) fn qualified(review: &Review) -> String {
    format!(
        "{}.{}",
        crate::quote_double(&review.payload.schema),
        crate::quote_double(&review.payload.table)
    )
}

#[cfg(feature = "isolated-profile")]
pub(crate) fn import_columns(
    schema: &str,
    table: &str,
    names: &[&str],
    options: &CsvOptions,
) -> String {
    let columns = names
        .iter()
        .map(|n| crate::quote_double(n))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "COPY {} ({columns}) FROM STDIN WITH ({})",
        qualified_names(schema, table),
        csv_options(options, false)
    )
}
fn qualified_names(schema: &str, table: &str) -> String {
    format!(
        "{}.{}",
        crate::quote_double(schema),
        crate::quote_double(table)
    )
}

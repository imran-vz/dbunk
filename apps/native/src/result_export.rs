//! Bounded, host-neutral retained-result formatting. No retrieval or publication.
//! Callers must mark omitted rows, truncated cells and unfinished streams Partial.
//! CSV NULL tokens may collide with real text; TXT flattens CRLF/LF like the baseline.
use std::{collections::HashSet, fmt, fmt::Write};

pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;
const MAX_COLUMNS: usize = 1_024;
const MAX_ROWS: usize = 100_000;
const MAX_CELLS: usize = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// Clipboard TSV: no headings; tabs and CRLF/LF within cells become spaces.
    Tsv,
    Csv,
    Json,
    Sql,
    Html,
    Markdown,
    Txt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Completeness {
    Complete,
    /// Includes truncation of individual values and sources still being received.
    Partial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    CompleteResult,
    /// Explicit caller acknowledgement; the returned completeness stays Partial.
    RetainedRows,
}

pub struct ExportTable<'a> {
    pub columns: &'a [&'a str],
    pub rows: &'a [&'a [Option<String>]],
    /// Display-order source indices, paired with the projected headings above.
    /// None means rows already contain exactly the displayed columns.
    pub source_columns: Option<&'a [usize]>,
    pub completeness: Completeness,
}

impl ExportTable<'_> {
    /// Called only after validation; hidden cells are neither scanned nor copied.
    fn cells<'a>(&'a self, row: &'a [Option<String>]) -> impl Iterator<Item = &'a Option<String>> {
        (0..self.columns.len())
            .map(move |index| &row[self.source_columns.map_or(index, |columns| columns[index])])
    }
}

/// Components are unquoted identifiers, so a literal dot within a name is safe.
#[derive(Clone, Copy)]
pub struct SqlTarget<'a> {
    pub schema: Option<&'a str>,
    pub table: &'a str,
}

pub struct Options<'a> {
    pub format: Format,
    pub encoding: Encoding,
    pub scope: Scope,
    pub null_as: &'a str,
    pub sql_target: Option<SqlTarget<'a>>,
    pub pretty_json: bool,
}

impl Options<'_> {
    pub fn new(format: Format) -> Self {
        Self {
            format,
            encoding: Encoding::Utf8,
            scope: Scope::CompleteResult,
            null_as: if format == Format::Tsv { "NULL" } else { "" },
            sql_target: None,
            pretty_json: true,
        }
    }
}

#[derive(Debug)]
pub struct PreparedExport {
    pub bytes: Vec<u8>,
    pub completeness: Completeness,
    pub row_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportError {
    PartialSource,
    InputTooLarge,
    OutputTooLarge,
    InvalidRowWidth,
    InvalidProjection,
    DuplicateColumn,
    MissingSqlTarget,
    InvalidSqlIdentifier,
    SqlNulValue,
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PartialSource => "Result is partial; explicitly choose retained rows to export",
            Self::InputTooLarge => "Export input exceeds the bounded result limits",
            Self::OutputTooLarge => "Encoded export exceeds 8 MiB",
            Self::InvalidRowWidth => "Export row does not match its column count",
            Self::InvalidProjection => {
                "Export projection does not match its columns or source rows"
            }
            Self::DuplicateColumn => "JSON and SQL export require unique column names",
            Self::MissingSqlTarget => "SQL export requires a target table",
            Self::InvalidSqlIdentifier => "SQL export contains an empty or NUL identifier",
            Self::SqlNulValue => "PostgreSQL SQL text cannot contain a NUL value",
        })
    }
}

impl std::error::Error for ExportError {}

/// Measures escaped, encoded bytes before allocating the output buffer. Both
/// input and output are finite; UTF-16 includes its BOM in the same 8 MiB budget.
pub fn export(
    table: &ExportTable<'_>,
    options: &Options<'_>,
) -> Result<PreparedExport, ExportError> {
    validate(table, options)?;
    let bom_bytes = usize::from(options.encoding == Encoding::Utf16Le) * 2;
    let mut measure = Output {
        encoding: options.encoding,
        length: bom_bytes,
        bytes: None,
    };
    render(&mut measure, table, options).map_err(|_| ExportError::OutputTooLarge)?;
    let mut bytes = Vec::with_capacity(measure.length);
    if bom_bytes != 0 {
        bytes.extend_from_slice(&[0xff, 0xfe]);
    }
    let mut output = Output {
        encoding: options.encoding,
        length: bom_bytes,
        bytes: Some(bytes),
    };
    render(&mut output, table, options).map_err(|_| ExportError::OutputTooLarge)?;
    Ok(PreparedExport {
        bytes: output.bytes.unwrap_or_default(),
        completeness: table.completeness,
        row_count: table.rows.len(),
    })
}

fn validate(table: &ExportTable<'_>, options: &Options<'_>) -> Result<(), ExportError> {
    if table.completeness == Completeness::Partial && options.scope == Scope::CompleteResult {
        return Err(ExportError::PartialSource);
    }
    if table.columns.len() > MAX_COLUMNS
        || table.rows.len() > MAX_ROWS
        || table.columns.len().saturating_mul(table.rows.len()) > MAX_CELLS
    {
        return Err(ExportError::InputTooLarge);
    }
    if table
        .source_columns
        .is_some_and(|columns| columns.len() != table.columns.len())
    {
        return Err(ExportError::InvalidProjection);
    }
    let mut input_bytes = 0_usize;
    let mut count = |text: &str| {
        input_bytes = input_bytes.saturating_add(text.len());
        if input_bytes > MAX_EXPORT_BYTES {
            Err(ExportError::InputTooLarge)
        } else {
            Ok(())
        }
    };
    count(options.null_as)?;
    for column in table.columns {
        count(column)?;
    }
    for row in table.rows {
        match table.source_columns {
            Some(columns) if columns.iter().any(|index| *index >= row.len()) => {
                return Err(ExportError::InvalidProjection);
            }
            None if row.len() != table.columns.len() => return Err(ExportError::InvalidRowWidth),
            _ => {}
        }
        for cell in table.cells(row).flatten() {
            count(cell)?;
        }
    }
    if matches!(options.format, Format::Json | Format::Sql) {
        let mut unique = HashSet::with_capacity(table.columns.len());
        for column in table.columns {
            if !unique.insert(*column) {
                return Err(ExportError::DuplicateColumn);
            }
        }
    }
    if options.format == Format::Sql {
        let target = options.sql_target.ok_or(ExportError::MissingSqlTarget)?;
        for name in target.schema.into_iter().chain([target.table]) {
            count(name)?;
            sql_identifier_valid(name)?;
        }
        if table.columns.is_empty() {
            return Err(ExportError::InvalidSqlIdentifier);
        }
        for column in table.columns {
            sql_identifier_valid(column)?;
        }
        if table.rows.iter().any(|row| {
            table
                .cells(row)
                .flatten()
                .any(|value| value.as_bytes().contains(&0))
        }) {
            return Err(ExportError::SqlNulValue);
        }
    }
    Ok(())
}

fn sql_identifier_valid(name: &str) -> Result<(), ExportError> {
    if name.is_empty() || name.as_bytes().contains(&0) {
        Err(ExportError::InvalidSqlIdentifier)
    } else {
        Ok(())
    }
}

struct Output {
    encoding: Encoding,
    length: usize,
    bytes: Option<Vec<u8>>,
}

impl Write for Output {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let size = match self.encoding {
            Encoding::Utf8 => text.len(),
            Encoding::Utf16Le => text
                .encode_utf16()
                .count()
                .checked_mul(2)
                .ok_or(fmt::Error)?,
        };
        let new_length = self.length.checked_add(size).ok_or(fmt::Error)?;
        if new_length > MAX_EXPORT_BYTES {
            return Err(fmt::Error);
        }
        if let Some(bytes) = &mut self.bytes {
            match self.encoding {
                Encoding::Utf8 => bytes.extend_from_slice(text.as_bytes()),
                Encoding::Utf16Le => {
                    for unit in text.encode_utf16() {
                        bytes.extend_from_slice(&unit.to_le_bytes());
                    }
                }
            }
        }
        self.length = new_length;
        Ok(())
    }
}

fn render(out: &mut Output, table: &ExportTable<'_>, options: &Options<'_>) -> fmt::Result {
    match options.format {
        Format::Json => json(out, table, options.pretty_json),
        Format::Sql => sql(out, table, options.sql_target.ok_or(fmt::Error)?),
        Format::Html => html(out, table, options),
        Format::Tsv | Format::Csv | Format::Markdown | Format::Txt => {
            delimited(out, table, options)
        }
    }
}

fn quoted(out: &mut Output, value: &str, quote: char, escape_backslash: bool) -> fmt::Result {
    out.write_char(quote)?;
    for ch in value.chars() {
        if ch == quote || (escape_backslash && ch == '\\') {
            out.write_char(ch)?;
        }
        out.write_char(ch)?;
    }
    out.write_char(quote)
}

fn json_string(out: &mut Output, value: &str) -> fmt::Result {
    out.write_char('"')?;
    for ch in value.chars() {
        match ch {
            '"' => out.write_str("\\\"")?,
            '\\' => out.write_str("\\\\")?,
            '\n' => out.write_str("\\n")?,
            '\r' => out.write_str("\\r")?,
            '\t' => out.write_str("\\t")?,
            '\u{8}' => out.write_str("\\b")?,
            '\u{c}' => out.write_str("\\f")?,
            '\0'..='\u{1f}' => write!(out, "\\u{:04x}", u32::from(ch))?,
            _ => out.write_char(ch)?,
        }
    }
    out.write_char('"')
}

fn json(out: &mut Output, table: &ExportTable<'_>, pretty: bool) -> fmt::Result {
    out.write_char('[')?;
    for (row_index, row) in table.rows.iter().enumerate() {
        if row_index != 0 {
            out.write_char(',')?;
        }
        out.write_str(if pretty { "\n  {" } else { "{" })?;
        for (index, (column, cell)) in table.columns.iter().zip(table.cells(row)).enumerate() {
            if index != 0 {
                out.write_char(',')?;
            }
            if pretty {
                out.write_str("\n    ")?;
            }
            json_string(out, column)?;
            out.write_str(if pretty { ": " } else { ":" })?;
            match cell {
                Some(text) => json_string(out, text)?,
                None => out.write_str("null")?,
            }
        }
        if pretty && !table.columns.is_empty() {
            out.write_str("\n  ")?;
        }
        out.write_char('}')?;
    }
    if pretty && !table.rows.is_empty() {
        out.write_char('\n')?;
    }
    out.write_char(']')
}

fn sql(out: &mut Output, table: &ExportTable<'_>, target: SqlTarget<'_>) -> fmt::Result {
    for (row_index, row) in table.rows.iter().enumerate() {
        if row_index != 0 {
            out.write_char('\n')?;
        }
        out.write_str("INSERT INTO ")?;
        if let Some(schema) = target.schema {
            quoted(out, schema, '"', false)?;
            out.write_char('.')?;
        }
        quoted(out, target.table, '"', false)?;
        out.write_str(" (")?;
        for (index, column) in table.columns.iter().enumerate() {
            if index != 0 {
                out.write_str(", ")?;
            }
            quoted(out, column, '"', false)?;
        }
        out.write_str(") VALUES (")?;
        for (index, cell) in table.cells(row).enumerate() {
            if index != 0 {
                out.write_str(", ")?;
            }
            match cell {
                None => out.write_str("NULL")?,
                Some(value) => {
                    // E literals preserve backslashes regardless of the server's
                    // standard_conforming_strings setting.
                    let backslashes = value.contains('\\');
                    if backslashes {
                        out.write_char('E')?;
                    }
                    quoted(out, value, '\'', backslashes)?;
                }
            }
        }
        out.write_str(");")?;
    }
    Ok(())
}

fn html_text(out: &mut Output, text: &str) -> fmt::Result {
    for ch in text.chars() {
        match ch {
            '&' => out.write_str("&amp;")?,
            '<' => out.write_str("&lt;")?,
            '>' => out.write_str("&gt;")?,
            '"' => out.write_str("&quot;")?,
            _ => out.write_char(ch)?,
        }
    }
    Ok(())
}

fn html(out: &mut Output, table: &ExportTable<'_>, options: &Options<'_>) -> fmt::Result {
    out.write_str("<!doctype html>\n<html>\n<head><meta charset=\"")?;
    out.write_str(match options.encoding {
        Encoding::Utf8 => "utf-8",
        Encoding::Utf16Le => "utf-16le",
    })?;
    out.write_str("\"><title>dbunk export</title></head>\n<body><table>\n<thead><tr>")?;
    for column in table.columns {
        out.write_str("<th>")?;
        html_text(out, column)?;
        out.write_str("</th>")?;
    }
    out.write_str("</tr></thead>\n<tbody>\n")?;
    for (index, row) in table.rows.iter().enumerate() {
        if index != 0 {
            out.write_char('\n')?;
        }
        out.write_str("<tr>")?;
        for cell in table.cells(row) {
            out.write_str("<td>")?;
            html_text(out, cell.as_deref().unwrap_or(options.null_as))?;
            out.write_str("</td>")?;
        }
        out.write_str("</tr>")?;
    }
    out.write_str("\n</tbody>\n</table></body>\n</html>")
}

fn delimited(out: &mut Output, table: &ExportTable<'_>, options: &Options<'_>) -> fmt::Result {
    if options.format != Format::Tsv {
        line(out, table.columns.iter().copied().map(Some), options, true)?;
    }
    if options.format == Format::Markdown {
        out.write_char('\n')?;
        line(
            out,
            table.columns.iter().map(|_| Some("---")),
            options,
            true,
        )?;
    }
    for (index, row) in table.rows.iter().enumerate() {
        if index != 0 || options.format != Format::Tsv {
            out.write_char('\n')?;
        }
        line(out, table.cells(row).map(Option::as_deref), options, false)?;
    }
    Ok(())
}

fn line<'a>(
    out: &mut Output,
    cells: impl Iterator<Item = Option<&'a str>>,
    options: &Options<'_>,
    header: bool,
) -> fmt::Result {
    let format = options.format;
    if format == Format::Markdown {
        out.write_str("| ")?;
    }
    for (index, cell) in cells.enumerate() {
        if index != 0 {
            out.write_str(match format {
                Format::Csv => ",",
                Format::Markdown => " | ",
                _ => "\t",
            })?;
        }
        let is_null = cell.is_none();
        let cell = cell.unwrap_or(options.null_as);
        match format {
            Format::Csv if cell.contains([',', '"', '\r', '\n']) => quoted(out, cell, '"', false)?,
            Format::Markdown => flattened(out, cell, format)?,
            Format::Txt | Format::Tsv if !header && !is_null => flattened(out, cell, format)?,
            _ => out.write_str(cell)?,
        }
    }
    if format == Format::Markdown {
        out.write_str(" |")?;
    }
    Ok(())
}

fn flattened(out: &mut Output, text: &str, format: Format) -> fmt::Result {
    let markdown = format == Format::Markdown;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        match ch {
            '\n' => out.write_str(if markdown { "<br>" } else { " " })?,
            '\t' if format == Format::Tsv => out.write_char(' ')?,
            '\\' | '|' if markdown => {
                out.write_char('\\')?;
                out.write_char(ch)?;
            }
            _ => out.write_char(ch)?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(
        columns: &[&str],
        rows: &[Vec<Option<String>>],
        options: &Options<'_>,
    ) -> Result<PreparedExport, ExportError> {
        let rows = rows.iter().map(Vec::as_slice).collect::<Vec<_>>();
        export(
            &ExportTable {
                columns,
                rows: &rows,
                source_columns: None,
                completeness: Completeness::Complete,
            },
            options,
        )
    }

    fn text(columns: &[&str], rows: &[Vec<Option<String>>], options: &Options<'_>) -> String {
        String::from_utf8(prepared(columns, rows, options).unwrap().bytes).unwrap()
    }

    fn cell(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    #[test]
    fn clipboard_tsv_omits_headings_and_flattens_tabs_and_newlines() {
        let options = Options::new(Format::Tsv);
        assert_eq!(
            text(
                &["a", "b", "c"],
                &[
                    vec![cell("x\ty\r\nz\nw\rq"), None, cell("")],
                    vec![cell("9007199254740993"), cell("NULL"), cell("🦀")],
                ],
                &options,
            ),
            "x y z w\rq\tNULL\t\n9007199254740993\tNULL\t🦀"
        );
        assert_eq!(text(&["a"], &[], &options), "");
    }

    #[test]
    fn projection_preserves_display_order_and_does_not_scan_hidden_cells() {
        let hidden = format!("\0{}", "x".repeat(MAX_EXPORT_BYTES));
        let source = [vec![cell("first"), Some(hidden), cell("last")]];
        let rows = [source[0].as_slice()];
        let table = ExportTable {
            columns: &["last", "first"],
            rows: &rows,
            source_columns: Some(&[2, 0]),
            completeness: Completeness::Complete,
        };
        for format in [
            Format::Tsv,
            Format::Csv,
            Format::Json,
            Format::Sql,
            Format::Html,
            Format::Markdown,
            Format::Txt,
        ] {
            let mut options = Options::new(format);
            options.sql_target = Some(SqlTarget {
                schema: Some("public"),
                table: "example",
            });
            let result = export(&table, &options).unwrap();
            let expected = prepared(
                &["last", "first"],
                &[vec![cell("last"), cell("first")]],
                &options,
            )
            .unwrap();
            assert_eq!(result.bytes, expected.bytes, "{format:?}");
        }
    }

    #[test]
    fn invalid_projection_refuses_before_indexing_or_omitting_selected_values() {
        let source = [vec![cell("a")], vec![]];
        let rows = [source[0].as_slice(), source[1].as_slice()];
        for projection in [&[0][..], &[usize::MAX][..], &[][..], &[0, 0][..]] {
            let table = ExportTable {
                columns: &["a"],
                rows: &rows,
                source_columns: Some(projection),
                completeness: Completeness::Complete,
            };
            assert_eq!(
                export(&table, &Options::new(Format::Tsv)).unwrap_err(),
                ExportError::InvalidProjection
            );
        }
    }

    #[test]
    fn csv_baseline_preserves_null_empty_quotes_and_exact_numeric_text() {
        let columns = ["id", "col, \"weird\"", "note"];
        let rows = [
            vec![cell("9007199254740993"), cell("Ada \"Countess\""), None],
            vec![cell("0001.2300"), cell(""), cell("a\r\nb\rc")],
        ];
        let mut options = Options::new(Format::Csv);
        options.null_as = "NULL";
        assert_eq!(
            text(&columns, &rows, &options),
            "id,\"col, \"\"weird\"\"\",note\n9007199254740993,\"Ada \"\"Countess\"\"\",NULL\n0001.2300,,\"a\r\nb\rc\""
        );
        options.null_as = "null,token";
        assert_eq!(text(&["a"], &[vec![None]], &options), "a\n\"null,token\"");
        assert_eq!(text(&["a"], &[], &options), "a");
    }

    #[test]
    fn json_preserves_strings_null_unicode_control_characters_and_order() {
        let columns = ["id", "__proto__", "name"];
        let rows = [vec![
            cell("9007199254740993"),
            None,
            cell("\"\\\n\r\t\u{8}\u{c}\0🦀"),
        ]];
        let mut options = Options::new(Format::Json);
        options.pretty_json = false;
        assert_eq!(
            text(&columns, &rows, &options),
            "[{\"id\":\"9007199254740993\",\"__proto__\":null,\"name\":\"\\\"\\\\\\n\\r\\t\\b\\f\\u0000🦀\"}]"
        );
        options.pretty_json = true;
        assert_eq!(
            text(&["id"], &[vec![cell("1")]], &options),
            "[\n  {\n    \"id\": \"1\"\n  }\n]"
        );
        assert_eq!(text(&["id"], &[], &options), "[]");
    }

    #[test]
    fn sql_baseline_quotes_identifiers_and_values_without_numeric_coercion() {
        let mut options = Options::new(Format::Sql);
        options.sql_target = Some(SqlTarget {
            schema: Some("public"),
            table: "people",
        });
        assert_eq!(
            text(
                &["id", "name", "note"],
                &[
                    vec![cell("1"), cell("Ada"), cell("hello")],
                    vec![cell("2"), cell("Grace"), None],
                ],
                &options
            ),
            "INSERT INTO \"public\".\"people\" (\"id\", \"name\", \"note\") VALUES ('1', 'Ada', 'hello');\nINSERT INTO \"public\".\"people\" (\"id\", \"name\", \"note\") VALUES ('2', 'Grace', NULL);"
        );
        options.sql_target = Some(SqlTarget {
            schema: Some("a.b"),
            table: "quo\"te",
        });
        assert_eq!(
            text(&["v\"al"], &[vec![cell("O'Reilly\\path")]], &options),
            "INSERT INTO \"a.b\".\"quo\"\"te\" (\"v\"\"al\") VALUES (E'O''Reilly\\\\path');"
        );
        assert_eq!(text(&["id"], &[], &options), "");
        assert_eq!(
            prepared(&["a"], &[vec![cell("bad\0")]], &options).unwrap_err(),
            ExportError::SqlNulValue
        );
        options.sql_target = Some(SqlTarget {
            schema: None,
            table: "",
        });
        assert_eq!(
            prepared(&["a"], &[], &options).unwrap_err(),
            ExportError::InvalidSqlIdentifier
        );
    }

    #[test]
    fn html_markdown_and_txt_match_baseline_escaping() {
        let rows = [vec![cell("<&>\""), None]];
        let mut options = Options::new(Format::Html);
        options.null_as = "NULL";
        assert_eq!(
            text(&["a&", "b"], &rows, &options),
            "<!doctype html>\n<html>\n<head><meta charset=\"utf-8\"><title>dbunk export</title></head>\n<body><table>\n<thead><tr><th>a&amp;</th><th>b</th></tr></thead>\n<tbody>\n<tr><td>&lt;&amp;&gt;&quot;</td><td>NULL</td></tr>\n</tbody>\n</table></body>\n</html>"
        );
        options.format = Format::Markdown;
        assert_eq!(
            text(&["a|", "b"], &[vec![cell("x\\y\r\nz\nq"), None]], &options),
            "| a\\| | b |\n| --- | --- |\n| x\\\\y<br>z<br>q | NULL |"
        );
        options.format = Format::Txt;
        assert_eq!(
            text(&["a", "b"], &[vec![cell("x\r\ny\nz\rq"), None]], &options),
            "a\tb\nx y z\rq\tNULL"
        );
        options.null_as = "null\ntoken";
        assert_eq!(text(&["a"], &[vec![None]], &options), "a\nnull\ntoken");
    }

    #[test]
    fn partial_sources_require_explicit_retained_scope_and_remain_partial() {
        let rows = [vec![cell("truncated…")]];
        let refs = [rows[0].as_slice()];
        let table = ExportTable {
            columns: &["value"],
            rows: &refs,
            source_columns: None,
            completeness: Completeness::Partial,
        };
        let mut options = Options::new(Format::Csv);
        assert_eq!(
            export(&table, &options).unwrap_err(),
            ExportError::PartialSource
        );
        options.scope = Scope::RetainedRows;
        let result = export(&table, &options).unwrap();
        assert_eq!(result.completeness, Completeness::Partial);
        assert_eq!(result.row_count, 1);
        assert_eq!(result.bytes, "value\ntruncated…".as_bytes());
    }

    #[test]
    fn duplicate_headings_and_ragged_rows_cannot_silently_drop_values() {
        for format in [Format::Json, Format::Sql] {
            assert_eq!(
                prepared(
                    &["a", "a"],
                    &[vec![cell("1"), cell("2")]],
                    &Options::new(format)
                )
                .unwrap_err(),
                ExportError::DuplicateColumn
            );
        }
        assert_eq!(
            text(
                &["a", "a"],
                &[vec![cell("1"), cell("2")]],
                &Options::new(Format::Csv)
            ),
            "a,a\n1,2"
        );
        for format in [
            Format::Csv,
            Format::Json,
            Format::Sql,
            Format::Html,
            Format::Markdown,
            Format::Txt,
        ] {
            assert_eq!(
                prepared(&["a"], &[vec![cell("1"), cell("2")]], &Options::new(format)).unwrap_err(),
                ExportError::InvalidRowWidth
            );
            assert_eq!(
                prepared(&["a"], &[vec![]], &Options::new(format)).unwrap_err(),
                ExportError::InvalidRowWidth
            );
        }
    }

    #[test]
    fn utf16_includes_bom_surrogate_pairs_and_correct_html_metadata() {
        let mut options = Options::new(Format::Csv);
        options.encoding = Encoding::Utf16Le;
        assert_eq!(
            prepared(&["a"], &[vec![cell("🦀")]], &options)
                .unwrap()
                .bytes,
            [0xff, 0xfe, b'a', 0, b'\n', 0, 0x3e, 0xd8, 0x80, 0xdd]
        );
        options.format = Format::Html;
        let bytes = prepared(&["a"], &[], &options).unwrap().bytes;
        let units = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect::<Vec<_>>();
        assert!(
            String::from_utf16(&units)
                .unwrap()
                .contains("charset=\"utf-16le\"")
        );
    }

    #[test]
    fn budgets_account_for_escape_expansion_encoding_and_repeated_null_tokens() {
        let options = Options::new(Format::Csv);
        assert_eq!(
            prepared(
                &["a"],
                &[vec![cell(&"x".repeat(MAX_EXPORT_BYTES))]],
                &options
            )
            .unwrap_err(),
            ExportError::InputTooLarge
        );
        let output = prepared(
            &["a"],
            &[vec![cell(&"x".repeat(MAX_EXPORT_BYTES - 2))]],
            &options,
        )
        .unwrap();
        assert_eq!(output.bytes.len(), MAX_EXPORT_BYTES);
        assert_eq!(
            prepared(
                &["a"],
                &[vec![cell(&"\"".repeat(MAX_EXPORT_BYTES / 2))]],
                &options
            )
            .unwrap_err(),
            ExportError::OutputTooLarge
        );
        let mut options = Options::new(Format::Csv);
        options.encoding = Encoding::Utf16Le;
        assert_eq!(
            prepared(
                &["a"],
                &[vec![cell(&"x".repeat(MAX_EXPORT_BYTES / 2))]],
                &options
            )
            .unwrap_err(),
            ExportError::OutputTooLarge
        );
        options.encoding = Encoding::Utf8;
        let null_token = "x".repeat(MAX_EXPORT_BYTES / 2);
        options.null_as = &null_token;
        assert_eq!(
            prepared(&["a"], &[vec![None], vec![None]], &options).unwrap_err(),
            ExportError::OutputTooLarge
        );
    }
}

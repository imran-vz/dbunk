use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};

pub(crate) const MAX_FIELD_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RECORD_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_COLUMNS: usize = 1_600;
pub(crate) const CSV_OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
const MAX_NULL_TOKEN_BYTES: usize = 1024;
const UTF8_BOM: [u8; 3] = [0xef, 0xbb, 0xbf];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CsvOptions {
    pub delimiter: String,
    pub quote: String,
    pub escape: String,
    pub null_token: String,
    pub header: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: ",".into(),
            quote: "\"".into(),
            escape: "\"".into(),
            null_token: "\\N".into(),
            header: true,
        }
    }
}

impl CsvOptions {
    pub(crate) fn validate(&self) -> Result<(), CsvError> {
        let delimiter = validate_control_byte("delimiter", &self.delimiter)?;
        let quote = validate_control_byte("quote", &self.quote)?;
        let _escape = validate_control_byte("escape", &self.escape)?;

        if delimiter == quote {
            return Err(CsvError::configuration(
                "delimiter and quote must be different",
            ));
        }
        if self.null_token.len() > MAX_NULL_TOKEN_BYTES {
            return Err(CsvError::configuration(
                "NULL token exceeds the 1 KiB limit",
            ));
        }
        if self.null_token.as_bytes().iter().any(|byte| {
            matches!(*byte, b'\0' | b'\r' | b'\n') || *byte == delimiter || *byte == quote
        }) {
            return Err(CsvError::configuration(
                "NULL token cannot contain NUL, the delimiter, quote, carriage return, or line feed",
            ));
        }

        Ok(())
    }

    fn control_bytes(&self) -> Result<ControlBytes, CsvError> {
        self.validate()?;
        Ok(ControlBytes {
            delimiter: self.delimiter.as_bytes()[0],
            quote: self.quote.as_bytes()[0],
            escape: self.escape.as_bytes()[0],
        })
    }
}

fn validate_control_byte(name: &str, value: &str) -> Result<u8, CsvError> {
    let [byte] = value.as_bytes() else {
        return Err(CsvError::configuration(format!(
            "{name} must be exactly one UTF-8 byte"
        )));
    };
    if matches!(*byte, b'\0' | b'\r' | b'\n') {
        return Err(CsvError::configuration(format!(
            "{name} cannot be NUL, carriage return, or line feed"
        )));
    }
    Ok(*byte)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CsvError {
    pub record: u64,
    pub column: Option<usize>,
    pub reason: String,
}

impl CsvError {
    fn configuration(reason: impl Into<String>) -> Self {
        Self {
            record: 0,
            column: None,
            reason: reason.into(),
        }
    }

    fn at(record: u64, column: Option<usize>, reason: impl Into<String>) -> Self {
        Self {
            record,
            column,
            reason: reason.into(),
        }
    }
}

impl fmt::Display for CsvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.record, self.column) {
            (0, _) => formatter.write_str(&self.reason),
            (record, Some(column)) => {
                write!(
                    formatter,
                    "CSV record {record}, column {column}: {}",
                    self.reason
                )
            }
            (record, None) => write!(formatter, "CSV record {record}: {}", self.reason),
        }
    }
}

impl Error for CsvError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CsvCell {
    pub value: String,
    pub quoted: bool,
}

impl CsvCell {
    /// PostgreSQL applies the NULL marker only to an unquoted field.
    pub(crate) fn is_null(&self, options: &CsvOptions) -> bool {
        !self.quoted && self.value == options.null_token
    }
}

#[derive(Debug, Clone, Copy)]
struct ControlBytes {
    delimiter: u8,
    quote: u8,
    escape: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FieldState {
    Start,
    Unquoted,
    Quoted,
    Escaped,
    AfterQuote,
}

/// Incremental UTF-8 CSV parser. Every successful `push` emits at most one
/// complete logical record, including records with quoted line endings.
pub(crate) struct Parser {
    controls: ControlBytes,
    record_number: u64,
    record_bytes: usize,
    fields: Vec<CsvCell>,
    field: Vec<u8>,
    field_quoted: bool,
    field_state: FieldState,
    record_started: bool,
    pending_cr: bool,
    bom_prefix: Vec<u8>,
    bom_resolved: bool,
    failed: bool,
    finished: bool,
}

impl Parser {
    pub(crate) fn new(options: &CsvOptions) -> Result<Self, CsvError> {
        Ok(Self {
            controls: options.control_bytes()?,
            record_number: 1,
            record_bytes: 0,
            fields: Vec::new(),
            field: Vec::new(),
            field_quoted: false,
            field_state: FieldState::Start,
            record_started: false,
            pending_cr: false,
            bom_prefix: Vec::with_capacity(UTF8_BOM.len()),
            bom_resolved: false,
            failed: false,
            finished: false,
        })
    }

    pub(crate) fn push(&mut self, byte: u8) -> Result<Option<Vec<CsvCell>>, CsvError> {
        if self.finished {
            return Err(self.error(None, "parser has already finished"));
        }
        if self.failed {
            return Err(self.error(None, "parser is no longer usable after an error"));
        }

        let result = self.push_with_bom(byte);
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub(crate) fn finish(&mut self) -> Result<Option<Vec<CsvCell>>, CsvError> {
        if self.finished {
            return Err(self.error(None, "parser has already finished"));
        }
        if self.failed {
            return Err(self.error(None, "parser is no longer usable after an error"));
        }
        self.finished = true;

        let result = self.finish_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    /// True when a bounded preview ended after bytes belonging to an incomplete
    /// record. A consumed BOM by itself is not a record.
    pub(crate) fn has_pending_input(&self) -> bool {
        !self.bom_prefix.is_empty()
            || self.pending_cr
            || self.record_started
            || !self.fields.is_empty()
            || !self.field.is_empty()
            || self.field_state != FieldState::Start
    }

    pub(crate) fn next_record_number(&self) -> u64 {
        self.record_number
    }

    fn push_with_bom(&mut self, byte: u8) -> Result<Option<Vec<CsvCell>>, CsvError> {
        if self.bom_resolved {
            return self.push_data_byte(byte);
        }

        let prefix_index = self.bom_prefix.len();
        if byte == UTF8_BOM[prefix_index] {
            self.bom_prefix.push(byte);
            if self.bom_prefix.len() == UTF8_BOM.len() {
                self.bom_prefix.clear();
                self.bom_resolved = true;
            }
            return Ok(None);
        }

        self.bom_resolved = true;
        let prefix = std::mem::take(&mut self.bom_prefix);
        for prefix_byte in prefix {
            // A partial BOM contains no CSV control byte, so it cannot emit a row.
            let emitted = self.push_data_byte(prefix_byte)?;
            debug_assert!(emitted.is_none());
        }
        self.push_data_byte(byte)
    }

    fn finish_inner(&mut self) -> Result<Option<Vec<CsvCell>>, CsvError> {
        if !self.bom_resolved {
            self.bom_resolved = true;
            let prefix = std::mem::take(&mut self.bom_prefix);
            for prefix_byte in prefix {
                let emitted = self.push_data_byte(prefix_byte)?;
                debug_assert!(emitted.is_none());
            }
        }

        if self.pending_cr {
            return Err(self.error(
                Some(self.current_column()),
                "carriage return must be followed by line feed",
            ));
        }
        match self.field_state {
            FieldState::Quoted => {
                return Err(self.error(Some(self.current_column()), "unterminated quoted field"));
            }
            FieldState::Escaped => {
                return Err(self.error(
                    Some(self.current_column()),
                    "unterminated escape in quoted field",
                ));
            }
            FieldState::Start if !self.record_started && self.fields.is_empty() => return Ok(None),
            FieldState::Start | FieldState::Unquoted | FieldState::AfterQuote => {}
        }

        self.complete_record()
    }

    fn push_data_byte(&mut self, byte: u8) -> Result<Option<Vec<CsvCell>>, CsvError> {
        if self.pending_cr {
            if byte != b'\n' {
                return Err(self.error(
                    Some(self.current_column()),
                    "carriage return must be followed by line feed",
                ));
            }
            self.pending_cr = false;
            return self.complete_record();
        }

        if matches!(self.field_state, FieldState::Quoted | FieldState::Escaped) {
            self.count_record_byte()?;
            return self.push_quoted_byte(byte);
        }

        match byte {
            b'\n' => self.complete_record(),
            b'\r' => {
                self.pending_cr = true;
                Ok(None)
            }
            _ => {
                self.count_record_byte()?;
                self.push_unquoted_byte(byte)
            }
        }
    }

    fn push_quoted_byte(&mut self, byte: u8) -> Result<Option<Vec<CsvCell>>, CsvError> {
        match self.field_state {
            FieldState::Quoted
                if self.controls.escape != self.controls.quote && byte == self.controls.escape =>
            {
                self.field_state = FieldState::Escaped;
            }
            FieldState::Quoted if byte == self.controls.quote => {
                self.field_state = FieldState::AfterQuote;
            }
            FieldState::Quoted => self.push_field_byte(byte)?,
            FieldState::Escaped if byte == self.controls.quote || byte == self.controls.escape => {
                self.push_field_byte(byte)?;
                self.field_state = FieldState::Quoted;
            }
            FieldState::Escaped => {
                // PostgreSQL treats a distinct escape as ordinary data unless
                // it precedes the configured quote or escape byte.
                self.push_field_byte(self.controls.escape)?;
                self.push_field_byte(byte)?;
                self.field_state = FieldState::Quoted;
            }
            _ => unreachable!("quoted byte handling requires a quoted state"),
        }
        Ok(None)
    }

    fn push_unquoted_byte(&mut self, byte: u8) -> Result<Option<Vec<CsvCell>>, CsvError> {
        match self.field_state {
            FieldState::Start if byte == self.controls.delimiter => {
                self.finish_field()?;
                self.ensure_another_column()?;
            }
            FieldState::Start if byte == self.controls.quote => {
                self.field_quoted = true;
                self.field_state = FieldState::Quoted;
            }
            FieldState::Start => {
                self.push_field_byte(byte)?;
                self.field_state = FieldState::Unquoted;
            }
            FieldState::Unquoted if byte == self.controls.delimiter => {
                self.finish_field()?;
                self.ensure_another_column()?;
            }
            FieldState::Unquoted if byte == self.controls.quote => {
                return Err(self.error(
                    Some(self.current_column()),
                    "unexpected quote in unquoted field",
                ));
            }
            FieldState::Unquoted => self.push_field_byte(byte)?,
            FieldState::AfterQuote
                if self.controls.escape == self.controls.quote && byte == self.controls.quote =>
            {
                self.push_field_byte(byte)?;
                self.field_state = FieldState::Quoted;
            }
            FieldState::AfterQuote if byte == self.controls.delimiter => {
                self.finish_field()?;
                self.ensure_another_column()?;
            }
            FieldState::AfterQuote => {
                return Err(self.error(
                    Some(self.current_column()),
                    "unexpected character after closing quote",
                ));
            }
            FieldState::Quoted | FieldState::Escaped => {
                unreachable!("quoted states are handled separately")
            }
        }
        Ok(None)
    }

    fn count_record_byte(&mut self) -> Result<(), CsvError> {
        if self.record_bytes == MAX_RECORD_BYTES {
            return Err(self.error(None, "record exceeds the 8 MiB limit"));
        }
        self.record_bytes += 1;
        self.record_started = true;
        Ok(())
    }

    fn push_field_byte(&mut self, byte: u8) -> Result<(), CsvError> {
        if self.field.len() == MAX_FIELD_BYTES {
            return Err(self.error(Some(self.current_column()), "field exceeds the 1 MiB limit"));
        }
        self.field.push(byte);
        Ok(())
    }

    fn finish_field(&mut self) -> Result<(), CsvError> {
        if self.fields.len() == MAX_COLUMNS {
            return Err(self.error(
                Some(MAX_COLUMNS + 1),
                "record exceeds the 1600-column limit",
            ));
        }
        let bytes = std::mem::take(&mut self.field);
        let value = String::from_utf8(bytes)
            .map_err(|_| self.error(Some(self.current_column()), "field contains invalid UTF-8"))?;
        self.fields.push(CsvCell {
            value,
            quoted: self.field_quoted,
        });
        self.field_quoted = false;
        self.field_state = FieldState::Start;
        Ok(())
    }

    fn ensure_another_column(&self) -> Result<(), CsvError> {
        if self.fields.len() == MAX_COLUMNS {
            return Err(self.error(
                Some(MAX_COLUMNS + 1),
                "record exceeds the 1600-column limit",
            ));
        }
        Ok(())
    }

    fn complete_record(&mut self) -> Result<Option<Vec<CsvCell>>, CsvError> {
        self.finish_field()?;
        let record = std::mem::take(&mut self.fields);
        self.record_number += 1;
        self.record_bytes = 0;
        self.record_started = false;
        self.pending_cr = false;
        Ok(Some(record))
    }

    fn current_column(&self) -> usize {
        self.fields.len() + 1
    }

    fn error(&self, column: Option<usize>, reason: impl Into<String>) -> CsvError {
        CsvError::at(self.record_number, column, reason)
    }
}

enum EncodedCellKind<'a> {
    Null {
        bytes: &'a [u8],
        offset: usize,
    },
    Value {
        bytes: &'a [u8],
        offset: usize,
        opened: bool,
        closed: bool,
    },
}

/// Iterator over one PostgreSQL CSV cell. Non-NULL values are always quoted,
/// which keeps empty strings and values equal to the NULL marker distinct.
pub(crate) struct EncodedCell<'a> {
    kind: EncodedCellKind<'a>,
    quote: u8,
    escape: u8,
}

pub(crate) fn encode_cell<'a>(
    value: Option<&'a str>,
    options: &'a CsvOptions,
) -> Result<EncodedCell<'a>, CsvError> {
    let controls = options.control_bytes()?;
    let kind = match value {
        None => EncodedCellKind::Null {
            bytes: options.null_token.as_bytes(),
            offset: 0,
        },
        Some(value) => {
            if value.len() > MAX_FIELD_BYTES {
                return Err(CsvError::configuration("field exceeds the 1 MiB limit"));
            }
            EncodedCellKind::Value {
                bytes: value.as_bytes(),
                offset: 0,
                opened: false,
                closed: false,
            }
        }
    };
    Ok(EncodedCell {
        kind,
        quote: controls.quote,
        escape: controls.escape,
    })
}

impl Iterator for EncodedCell<'_> {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.kind {
            EncodedCellKind::Null { bytes, offset } => {
                if *offset == bytes.len() {
                    return None;
                }
                let end = (*offset + CSV_OUTPUT_CHUNK_BYTES).min(bytes.len());
                let chunk = bytes[*offset..end].to_vec();
                *offset = end;
                Some(chunk)
            }
            EncodedCellKind::Value {
                bytes,
                offset,
                opened,
                closed,
            } => {
                if *closed {
                    return None;
                }

                let mut chunk = Vec::with_capacity(CSV_OUTPUT_CHUNK_BYTES.min(bytes.len() + 2));
                if !*opened {
                    chunk.push(self.quote);
                    *opened = true;
                }
                while *offset < bytes.len() {
                    let byte = bytes[*offset];
                    let escaped = byte == self.quote || byte == self.escape;
                    let needed = if escaped { 2 } else { 1 };
                    if chunk.len() + needed > CSV_OUTPUT_CHUNK_BYTES {
                        break;
                    }
                    if escaped {
                        chunk.push(self.escape);
                    }
                    chunk.push(byte);
                    *offset += 1;
                }
                if *offset == bytes.len() && chunk.len() < CSV_OUTPUT_CHUNK_BYTES {
                    chunk.push(self.quote);
                    *closed = true;
                }
                Some(chunk)
            }
        }
    }
}

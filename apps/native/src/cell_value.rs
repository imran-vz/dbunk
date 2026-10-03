//! Pure retained-value inspection and specialized-editor models. Successful
//! validation never applies a mutation. SQL NULL is an outer Option, not text.
use serde::{Deserialize, de::IgnoredAny};
use std::{fmt, fmt::Write};

pub const MAX_VALUE_BYTES: usize = 1024 * 1024;
pub const MAX_ARRAY_ITEMS: usize = 4096;
pub const MAX_JSON_DEPTH: usize = 64;
pub const MAX_HEX_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Json,
    Array,
    Geometry,
}

/// PostgreSQL's displayed type names, with the baseline registry's precedence.
/// Other types retain the ordinary literal editor; bytea is not a hex editor.
pub fn classify(data_type: Option<&str>) -> Option<Kind> {
    let name = data_type?;
    if name.len() > MAX_VALUE_BYTES {
        return None;
    }
    let name = name.trim();
    if name.eq_ignore_ascii_case("json") || name.eq_ignore_ascii_case("jsonb") {
        Some(Kind::Json)
    } else if name.ends_with("[]") {
        Some(Kind::Array)
    } else if ["geometry", "geography"].iter().any(|prefix| {
        name.eq_ignore_ascii_case(prefix)
            || name
                .get(..prefix.len())
                .is_some_and(|part| part.eq_ignore_ascii_case(prefix))
                && name.as_bytes().get(prefix.len()) == Some(&b'(')
    }) {
        Some(Kind::Geometry)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueError {
    InputTooLarge,
    OutputTooLarge,
    InvalidJson,
    JsonTooDeep,
    InvalidArray,
    UnsupportedArrayShape,
    TooManyArrayItems,
    NulArrayValue,
    UnrecognizedWktPrefix,
}

impl fmt::Display for ValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "Value exceeds the 1 MiB editor/inspector limit",
            Self::OutputTooLarge => "Formatted value exceeds 1 MiB; original text is unchanged",
            Self::InvalidJson => "Value is not valid JSON",
            Self::JsonTooDeep => "JSON nesting exceeds 64 levels; retain the original text",
            Self::InvalidArray => "Malformed PostgreSQL array literal; original text is unchanged",
            Self::UnsupportedArrayShape => {
                "Nested or dimension-prefixed arrays require the raw editor"
            }
            Self::TooManyArrayItems => "Array exceeds 4096 items; retain the original text",
            Self::NulArrayValue => "PostgreSQL array text cannot contain a NUL byte",
            Self::UnrecognizedWktPrefix => {
                "Expected a recognized WKT prefix; database validation is still required"
            }
        })
    }
}

impl std::error::Error for ValueError {}

fn check_input(text: &str) -> Result<(), ValueError> {
    if text.len() > MAX_VALUE_BYTES {
        Err(ValueError::InputTooLarge)
    } else {
        Ok(())
    }
}

/// Validates grammar without converting numbers to f64 or constructing a Value.
/// Empty text and uppercase NULL are not JSON and are never coerced to SQL NULL.
pub fn validate_json(text: &str) -> Result<(), ValueError> {
    check_input(text)?;
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    for byte in text.bytes() {
        if quoted {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                quoted = false;
            }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    depth += 1;
                    if depth > MAX_JSON_DEPTH {
                        return Err(ValueError::JsonTooDeep);
                    }
                }
                b']' | b'}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    let mut reader = serde_json::Deserializer::from_str(text);
    IgnoredAny::deserialize(&mut reader).map_err(|_| ValueError::InvalidJson)?;
    reader.end().map_err(|_| ValueError::InvalidJson)
}

/// Only insignificant whitespace changes. Number lexemes, string escapes,
/// duplicate object keys and key order survive byte-for-byte.
pub fn pretty_json(text: &str) -> Result<String, ValueError> {
    validate_json(text)?;
    bounded_text(|out| render_json(out, text))
}

fn render_json(out: &mut TextOutput, text: &str) -> fmt::Result {
    let mut chars = text.chars().peekable();
    let mut depth = 0_usize;
    let mut quoted = false;
    let mut escaped = false;
    let mut previous = None;
    while let Some(ch) = chars.next() {
        if quoted {
            out.write_char(ch)?;
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = false;
            }
            previous = Some(ch);
            continue;
        }
        if ch.is_ascii_whitespace() {
            continue;
        }
        match ch {
            '"' => {
                quoted = true;
                out.write_char(ch)?;
            }
            '[' | '{' => {
                out.write_char(ch)?;
                depth += 1;
                while chars.peek().is_some_and(char::is_ascii_whitespace) {
                    chars.next();
                }
                if !matches!(chars.peek(), Some(']' | '}')) {
                    newline(out, depth)?;
                }
            }
            ']' | '}' => {
                depth -= 1;
                if !matches!(previous, Some('[' | '{')) {
                    newline(out, depth)?;
                }
                out.write_char(ch)?;
            }
            ',' => {
                out.write_char(ch)?;
                newline(out, depth)?;
            }
            ':' => out.write_str(": ")?,
            _ => out.write_char(ch)?,
        }
        previous = Some(ch);
    }
    Ok(())
}

fn newline(out: &mut TextOutput, depth: usize) -> fmt::Result {
    out.write_char('\n')?;
    for _ in 0..depth {
        out.write_str("  ")?;
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub struct HexDump {
    pub text: String,
    pub shown_bytes: usize,
    pub total_bytes: usize,
    pub truncated: bool,
}

/// Dumps UTF-8 bytes of the retained textual representation, not decoded bytea.
/// The byte cap can split a multibyte character; hex represents those exact bytes.
pub fn hex_dump(text: &str) -> Result<HexDump, ValueError> {
    check_input(text)?;
    let shown_bytes = text.len().min(MAX_HEX_BYTES);
    let bytes = &text.as_bytes()[..shown_bytes];
    let rendered = bounded_text(|out| {
        if bytes.is_empty() {
            return out.write_str("(empty)");
        }
        for (line, chunk) in bytes.chunks(16).enumerate() {
            if line != 0 {
                out.write_char('\n')?;
            }
            write!(out, "{:08x}  ", line * 16)?;
            for index in 0..16 {
                if index != 0 {
                    out.write_char(' ')?;
                }
                if let Some(byte) = chunk.get(index) {
                    write!(out, "{byte:02x}")?;
                } else {
                    out.write_str("  ")?;
                }
            }
            out.write_str("  ")?;
            for byte in chunk {
                out.write_char(if (0x20..0x7f).contains(byte) {
                    char::from(*byte)
                } else {
                    '·'
                })?;
            }
        }
        Ok(())
    })?;
    Ok(HexDump {
        text: rendered,
        shown_bytes,
        total_bytes: text.len(),
        truncated: shown_bytes != text.len(),
    })
}

/// One-dimensional comma-delimited PostgreSQL arrays. SQL NULL elements are
/// None; quoted/escaped NULL, empty strings and quoted whitespace stay text.
/// Non-comma element types (notably box[]) must remain in the raw editor.
/// PostgreSQL array_nulls=on semantics are assumed, as in normal server output.
pub fn parse_array(text: &str) -> Result<Vec<Option<String>>, ValueError> {
    check_input(text)?;
    if text.contains('\0') {
        return Err(ValueError::NulArrayValue);
    }
    let mut chars = text.chars().peekable();
    skip_array_space(&mut chars);
    match chars.next() {
        Some('[') => return Err(ValueError::UnsupportedArrayShape),
        Some('{') => {}
        _ => return Err(ValueError::InvalidArray),
    }
    skip_array_space(&mut chars);
    let mut items = Vec::new();
    if chars.peek() == Some(&'}') {
        chars.next();
    } else {
        loop {
            if items.len() == MAX_ARRAY_ITEMS {
                return Err(ValueError::TooManyArrayItems);
            }
            items.push(parse_array_item(&mut chars)?);
            match chars.next() {
                Some(',') => skip_array_space(&mut chars),
                Some('}') => break,
                _ => return Err(ValueError::InvalidArray),
            }
        }
    }
    skip_array_space(&mut chars);
    if chars.next().is_some() {
        return Err(ValueError::InvalidArray);
    }
    Ok(items)
}

fn skip_array_space(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while chars.peek().is_some_and(char::is_ascii_whitespace) {
        chars.next();
    }
}

fn parse_array_item(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<Option<String>, ValueError> {
    if chars.peek() == Some(&'{') {
        return Err(ValueError::UnsupportedArrayShape);
    }
    let quoted = chars.peek() == Some(&'"');
    let mut value = String::new();
    if quoted {
        chars.next();
        loop {
            match chars.next() {
                Some('"') => break,
                Some('\\') => value.push(chars.next().ok_or(ValueError::InvalidArray)?),
                Some(ch) => value.push(ch),
                None => return Err(ValueError::InvalidArray),
            }
        }
        skip_array_space(chars);
        return Ok(Some(value));
    }
    let mut escaped = false;
    let mut significant_length = 0;
    while let Some(ch) = chars.peek().copied() {
        match ch {
            ',' | '}' => break,
            '{' => return Err(ValueError::UnsupportedArrayShape),
            '"' => return Err(ValueError::InvalidArray),
            '\\' => {
                chars.next();
                value.push(chars.next().ok_or(ValueError::InvalidArray)?);
                escaped = true;
                significant_length = value.len();
            }
            _ => {
                chars.next();
                value.push(ch);
                if !ch.is_ascii_whitespace() {
                    significant_length = value.len();
                }
            }
        }
    }
    value.truncate(significant_length);
    if value.is_empty() {
        return Err(ValueError::InvalidArray);
    }
    Ok(if !escaped && value.eq_ignore_ascii_case("NULL") {
        None
    } else {
        Some(value)
    })
}

/// Canonical comma-array literal, measured before output allocation. This never
/// mutates the supplied structured elements when a size or value check fails.
pub fn format_array(items: &[Option<String>]) -> Result<String, ValueError> {
    if items.len() > MAX_ARRAY_ITEMS {
        return Err(ValueError::TooManyArrayItems);
    }
    let mut input_bytes = 0_usize;
    for item in items.iter().flatten() {
        input_bytes = input_bytes.saturating_add(item.len());
        if input_bytes > MAX_VALUE_BYTES {
            return Err(ValueError::InputTooLarge);
        }
        if item.contains('\0') {
            return Err(ValueError::NulArrayValue);
        }
    }
    bounded_text(|out| {
        out.write_char('{')?;
        for (index, item) in items.iter().enumerate() {
            if index != 0 {
                out.write_char(',')?;
            }
            let Some(value) = item else {
                out.write_str("NULL")?;
                continue;
            };
            let quoted = value.is_empty()
                || value.eq_ignore_ascii_case("NULL")
                || value
                    .chars()
                    .any(|ch| ch.is_whitespace() || matches!(ch, ',' | '{' | '}' | '"' | '\\'));
            if quoted {
                out.write_char('"')?;
                for ch in value.chars() {
                    if matches!(ch, '"' | '\\') {
                        out.write_char('\\')?;
                    }
                    out.write_char(ch)?;
                }
                out.write_char('"')?;
            } else {
                out.write_str(value)?;
            }
        }
        out.write_char('}')
    })
}

/// Structured element edits are transactional. Opening and inspecting a value
/// produces no replacement literal; an explicit change canonicalizes it.
#[derive(Debug)]
pub struct ArrayElements {
    items: Vec<Option<String>>,
    literal: Option<String>,
}
impl ArrayElements {
    pub fn parse(text: &str) -> Result<Self, ValueError> {
        Ok(Self {
            items: parse_array(text)?,
            literal: None,
        })
    }
    pub fn items(&self) -> &[Option<String>] {
        &self.items
    }
    pub fn literal(&self) -> Option<&str> {
        self.literal.as_deref()
    }
    pub fn replace(&mut self, index: usize, value: Option<String>) -> Result<(), ValueError> {
        let item = self.items.get_mut(index).ok_or(ValueError::InvalidArray)?;
        if *item == value {
            return Ok(());
        }
        let previous = std::mem::replace(item, value);
        match format_array(&self.items) {
            Ok(literal) => {
                self.literal = Some(literal);
                Ok(())
            }
            Err(error) => {
                self.items[index] = previous;
                Err(error)
            }
        }
    }
    pub fn append(&mut self) -> Result<usize, ValueError> {
        if self.items.len() >= MAX_ARRAY_ITEMS {
            return Err(ValueError::TooManyArrayItems);
        }
        self.items.push(Some(String::new()));
        match format_array(&self.items) {
            Ok(literal) => {
                self.literal = Some(literal);
                Ok(self.items.len() - 1)
            }
            Err(error) => {
                self.items.pop();
                Err(error)
            }
        }
    }
    pub fn remove(&mut self, index: usize) -> Result<(), ValueError> {
        if index >= self.items.len() {
            return Err(ValueError::InvalidArray);
        }
        let previous = self.items.remove(index);
        match format_array(&self.items) {
            Ok(literal) => {
                self.literal = Some(literal);
                Ok(())
            }
            Err(error) => {
                self.items.insert(index, previous);
                Err(error)
            }
        }
    }
}

/// Matches the baseline's recognized (optional SRID=digits;) type + '(' prefix.
/// This is a UX heuristic, NOT WKT syntax, coordinate or topology validation.
/// It intentionally does not claim support for EMPTY or Z/M/ZM forms absent
/// from that baseline. Database validation remains authoritative.
pub fn check_wkt_prefix(text: &str) -> Result<(), ValueError> {
    check_input(text)?;
    let mut value = text.trim_start();
    if value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SRID="))
    {
        value = &value[5..];
        let digits = value.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || value.as_bytes().get(digits) != Some(&b';') {
            return Err(ValueError::UnrecognizedWktPrefix);
        }
        value = value[digits + 1..].trim_start();
    }
    let name_length = value.bytes().take_while(u8::is_ascii_alphabetic).count();
    let name = &value[..name_length];
    let known = [
        "POINT",
        "LINESTRING",
        "POLYGON",
        "MULTIPOINT",
        "MULTILINESTRING",
        "MULTIPOLYGON",
        "GEOMETRYCOLLECTION",
        "CIRCULARSTRING",
        "COMPOUNDCURVE",
        "CURVEPOLYGON",
        "MULTICURVE",
        "MULTISURFACE",
        "POLYHEDRALSURFACE",
        "TIN",
        "TRIANGLE",
    ]
    .iter()
    .any(|candidate| name.eq_ignore_ascii_case(candidate));
    if known && value[name_length..].trim_start().starts_with('(') {
        Ok(())
    } else {
        Err(ValueError::UnrecognizedWktPrefix)
    }
}

struct TextOutput {
    length: usize,
    text: Option<String>,
}

impl Write for TextOutput {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let length = self.length.checked_add(text.len()).ok_or(fmt::Error)?;
        if length > MAX_VALUE_BYTES {
            return Err(fmt::Error);
        }
        if let Some(output) = &mut self.text {
            output.push_str(text);
        }
        self.length = length;
        Ok(())
    }
}

fn bounded_text(render: impl Fn(&mut TextOutput) -> fmt::Result) -> Result<String, ValueError> {
    let mut measure = TextOutput {
        length: 0,
        text: None,
    };
    render(&mut measure).map_err(|_| ValueError::OutputTooLarge)?;
    let mut output = TextOutput {
        length: 0,
        text: Some(String::with_capacity(measure.length)),
    };
    render(&mut output).map_err(|_| ValueError::OutputTooLarge)?;
    Ok(output.text.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(value: &str) -> Option<String> {
        Some(value.to_owned())
    }

    #[test]
    fn classification_matches_baseline_types_without_changing_primitives() {
        for name in ["json", " JSONB "] {
            assert_eq!(classify(Some(name)), Some(Kind::Json));
        }
        for name in ["text[]", "integer[]", "jsonb[]", "custom.type[]"] {
            assert_eq!(classify(Some(name)), Some(Kind::Array));
        }
        for name in [
            "geometry",
            " GEOGRAPHY ",
            "geometry(Point,4326)",
            "geography(LineString)",
        ] {
            assert_eq!(classify(Some(name)), Some(Kind::Geometry));
        }
        for name in [
            "text",
            "integer",
            "uuid",
            "timestamp",
            "bytea",
            "geometry_extra",
            "🦀",
        ] {
            assert_eq!(classify(Some(name)), None);
        }
        assert_eq!(classify(None), None);
    }

    #[test]
    fn json_pretty_retains_numeric_lexemes_string_escapes_and_duplicate_keys() {
        let source = r#" { "n" : 90071992547409931234567890, "n":-0.0000e+99999,"s":"\u0061 🦀 \" { } \\ \n", "a": [true,null,{},[]] } "#;
        let expected = "{\n  \"n\": 90071992547409931234567890,\n  \"n\": -0.0000e+99999,\n  \"s\": \"\\u0061 🦀 \\\" { } \\\\ \\n\",\n  \"a\": [\n    true,\n    null,\n    {},\n    []\n  ]\n}";
        assert_eq!(pretty_json(source).unwrap(), expected);
        assert_eq!(
            pretty_json(&pretty_json(source).unwrap()).unwrap(),
            expected
        );
        assert_eq!(pretty_json(" 1e999999 ").unwrap(), "1e999999");
        assert_eq!(pretty_json(" \"NULL\" ").unwrap(), "\"NULL\"");
        assert_eq!(pretty_json(" null ").unwrap(), "null");
    }

    #[test]
    fn json_invalid_deep_or_expanded_values_refuse_atomically() {
        for source in [
            "",
            "NULL",
            "{} trailing",
            "[1,]",
            "{unquoted:1}",
            "\"bad\nstring\"",
            "01",
            "NaN",
        ] {
            assert_eq!(
                validate_json(source),
                Err(ValueError::InvalidJson),
                "{source}"
            );
        }
        let nested = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        assert_eq!(pretty_json(&nested), Err(ValueError::JsonTooDeep));
        assert!(
            pretty_json(&format!(
                "{}0{}",
                "[".repeat(MAX_JSON_DEPTH),
                "]".repeat(MAX_JSON_DEPTH)
            ))
            .is_ok()
        );
        let large = format!("[{}0]", "0,".repeat(MAX_VALUE_BYTES / 3));
        assert!(large.len() < MAX_VALUE_BYTES);
        assert_eq!(pretty_json(&large), Err(ValueError::OutputTooLarge));
        assert!(large.starts_with("[0,0,"));
        assert_eq!(
            validate_json(&" ".repeat(MAX_VALUE_BYTES + 1)),
            Err(ValueError::InputTooLarge)
        );
    }

    #[test]
    fn hex_matches_baseline_utf8_layout_and_reports_exact_truncation() {
        let dump = hex_dump("A\0é🦀").unwrap();
        assert_eq!(
            dump.text,
            format!("00000000  {:47}  A·······", "41 00 c3 a9 f0 9f a6 80")
        );
        assert_eq!(
            (dump.shown_bytes, dump.total_bytes, dump.truncated),
            (8, 8, false)
        );
        assert_eq!(hex_dump("").unwrap().text, "(empty)");
        let source = format!("{}🦀", "a".repeat(MAX_HEX_BYTES - 1));
        let dump = hex_dump(&source).unwrap();
        assert_eq!(
            (dump.shown_bytes, dump.total_bytes, dump.truncated),
            (4096, 4099, true)
        );
        assert!(dump.text.lines().last().unwrap().contains("61 f0  "));
        assert_eq!(dump.text.lines().count(), 256);
        assert!(!hex_dump(&"x".repeat(MAX_HEX_BYTES)).unwrap().truncated);
        assert_eq!(
            hex_dump(&"x".repeat(MAX_VALUE_BYTES + 1)),
            Err(ValueError::InputTooLarge)
        );
    }

    #[test]
    fn arrays_preserve_null_empty_quoted_whitespace_escapes_and_unicode() {
        let source = r#" { NULL, "NULL", null, "", "  spaced  ", "a,b", "a\"b", "c\\d", "🦀", a b, N\ULL, \ leading\  } "#;
        let expected = vec![
            None,
            text("NULL"),
            None,
            text(""),
            text("  spaced  "),
            text("a,b"),
            text("a\"b"),
            text("c\\d"),
            text("🦀"),
            text("a b"),
            text("NULL"),
            text(" leading "),
        ];
        assert_eq!(parse_array(source).unwrap(), expected);
        let formatted = format_array(&expected).unwrap();
        assert_eq!(
            formatted,
            r#"{NULL,"NULL",NULL,"","  spaced  ","a,b","a\"b","c\\d",🦀,"a b","NULL"," leading "}"#
        );
        assert_eq!(parse_array(&formatted).unwrap(), expected);
        assert_eq!(parse_array(" { } ").unwrap(), Vec::<Option<String>>::new());
        assert_eq!(format_array(&[]).unwrap(), "{}");
        assert_eq!(
            parse_array("{a,b,c}").unwrap(),
            vec![text("a"), text("b"), text("c")]
        );
    }

    #[test]
    fn unsupported_or_malformed_arrays_never_flatten_or_rewrite_input() {
        for source in ["{{a,b},{c,d}}", "[0:1]={a,b}", "{a,{b}}"] {
            assert_eq!(parse_array(source), Err(ValueError::UnsupportedArrayShape));
        }
        for source in [
            "plain",
            "",
            "{a,}",
            "{,a}",
            "{a,,b}",
            "{\"unterminated}",
            "{\"a\"tail}",
            "{a}tail",
            "{a\\",
            "{a\"b}",
        ] {
            assert_eq!(
                parse_array(source),
                Err(ValueError::InvalidArray),
                "{source}"
            );
        }
        assert_eq!(parse_array("{\"{a,b}\"}").unwrap(), vec![text("{a,b}")]);
        assert_eq!(parse_array("{a\0b}"), Err(ValueError::NulArrayValue));
    }

    #[test]
    fn array_item_and_byte_budgets_refuse_without_mutating_elements() {
        let at_limit = vec![None; MAX_ARRAY_ITEMS];
        assert_eq!(
            parse_array(&format_array(&at_limit).unwrap()).unwrap(),
            at_limit
        );
        assert_eq!(
            format_array(&vec![None; MAX_ARRAY_ITEMS + 1]),
            Err(ValueError::TooManyArrayItems)
        );
        assert_eq!(
            parse_array(&format!("{{{}a}}", "a,".repeat(MAX_ARRAY_ITEMS))),
            Err(ValueError::TooManyArrayItems)
        );
        let escaped = vec![text(&"\\".repeat(MAX_VALUE_BYTES / 2))];
        assert_eq!(format_array(&escaped), Err(ValueError::OutputTooLarge));
        assert_eq!(escaped[0].as_ref().unwrap().len(), MAX_VALUE_BYTES / 2);
        assert_eq!(format_array(&[text("\0")]), Err(ValueError::NulArrayValue));
        assert_eq!(
            parse_array(&"x".repeat(MAX_VALUE_BYTES + 1)),
            Err(ValueError::InputTooLarge)
        );
    }

    #[test]
    fn element_edits_preserve_original_until_changed_and_distinguish_null_text_empty() {
        let original = r#" {NULL, "NULL", "", " a,b ", "🦀"} "#;
        let mut elements = ArrayElements::parse(original).unwrap();
        assert_eq!(elements.literal(), None);
        elements.replace(1, text("NULL")).unwrap();
        assert_eq!(elements.literal(), None);
        let index = elements.append().unwrap();
        elements.replace(index, text("new\nvalue")).unwrap();
        elements.replace(0, text("NULL")).unwrap();
        elements.replace(1, None).unwrap();
        elements.remove(4).unwrap();
        let expected = vec![
            text("NULL"),
            None,
            text(""),
            text(" a,b "),
            text("new\nvalue"),
        ];
        assert_eq!(elements.items(), expected);
        assert_eq!(parse_array(elements.literal().unwrap()).unwrap(), expected);
        while !elements.items().is_empty() {
            elements.remove(0).unwrap();
        }
        assert_eq!(elements.literal(), Some("{}"));
    }
    #[test]
    fn refused_element_edits_preserve_items_and_last_literal() {
        let mut elements = ArrayElements::parse("{kept}").unwrap();
        assert_eq!(
            elements.replace(0, text("\0")),
            Err(ValueError::NulArrayValue)
        );
        assert_eq!(elements.items(), &[text("kept")]);
        assert_eq!(elements.literal(), None);
        elements.replace(0, text("changed")).unwrap();
        assert_eq!(
            elements.replace(0, text(&"\\".repeat(MAX_VALUE_BYTES / 2))),
            Err(ValueError::OutputTooLarge)
        );
        assert_eq!(elements.items(), &[text("changed")]);
        assert_eq!(elements.literal(), Some("{changed}"));
        let mut full =
            ArrayElements::parse(&format_array(&vec![None; MAX_ARRAY_ITEMS]).unwrap()).unwrap();
        assert_eq!(full.append(), Err(ValueError::TooManyArrayItems));
        assert_eq!(full.items().len(), MAX_ARRAY_ITEMS);
        assert_eq!(full.literal(), None);
    }
    #[test]
    fn geometry_check_is_explicitly_only_the_baseline_prefix_heuristic() {
        for source in [
            "POINT(10 20)",
            " srid=4326; polygon ((0 0,1 1,0 0))",
            "TRIANGLE((0 0))",
            "POINT(nonsense",
        ] {
            assert_eq!(check_wkt_prefix(source), Ok(()), "{source}");
        }
        for source in [
            "not a polygon",
            "POINT EMPTY",
            "POINT Z(1 2 3)",
            "POINTLESS(1 2)",
            "SRID=;POINT(1 2)",
            "NULL",
            "",
        ] {
            assert_eq!(
                check_wkt_prefix(source),
                Err(ValueError::UnrecognizedWktPrefix),
                "{source}"
            );
        }
        assert_eq!(
            check_wkt_prefix(&" ".repeat(MAX_VALUE_BYTES + 1)),
            Err(ValueError::InputTooLarge)
        );
    }
}

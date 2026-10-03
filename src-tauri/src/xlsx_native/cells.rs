use super::{xml, XlsxError};
use quick_xml::events::{BytesStart, Event};
use std::{borrow::Cow, sync::atomic::AtomicBool};

pub(super) struct Cell<'a> {
    pub row: u32,
    pub col: u32,
    pub value: Cow<'a, str>,
    pub formula: bool,
}
struct Pending {
    row: u32,
    col: u32,
    kind: String,
    value: String,
    has_value: bool,
    formula: bool,
}

/// Reads populated cells only, in strict row/column order. Missing references use
/// OOXML's sequential row/column positions; malformed/repeated positions refuse.
pub(super) fn scan<'a>(
    bytes: &[u8],
    strings: &'a [String],
    cancel: &AtomicBool,
    mut cell: impl FnMut(Cell<'a>) -> Result<(), XlsxError>,
) -> Result<(), XlsxError> {
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut pending: Option<Pending> = None;
    let mut row = 0;
    let mut next_col = 1;
    let mut last = (0, 0);
    let mut root = false;
    let mut sheet_data = false;
    xml::walk(bytes, cancel, |reader, event| {
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.local_name().into_inner();
                if path.get(1).map(Vec::as_slice) == Some(b"sheetData".as_slice()) {
                    let allowed = match path.last().map(Vec::as_slice) {
                        Some(b"sheetData") => name == b"row",
                        Some(b"row") => name == b"c",
                        Some(b"c") => matches!(name, b"v" | b"f" | b"is"),
                        Some(b"v" | b"f" | b"t") => false,
                        Some(b"is") => matches!(name, b"t" | b"r" | b"rPh" | b"phoneticPr"),
                        Some(b"r") => matches!(name, b"rPr" | b"t"),
                        _ => true,
                    };
                    if !allowed {
                        return Err(XlsxError::UnsupportedCell);
                    }
                }
                if path.is_empty() {
                    if name != b"worksheet" || !xml::in_ns(reader, &e, xml::MAIN) || root {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    root = true;
                }
                if name == b"sheetData" && path.as_slice() == [b"worksheet".as_slice()] {
                    if sheet_data || !xml::in_ns(reader, &e, xml::MAIN) {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    sheet_data = true;
                }
                if path.get(1).map(Vec::as_slice) == Some(b"sheetData".as_slice())
                    && !xml::in_ns(reader, &e, xml::MAIN)
                {
                    return Err(XlsxError::InvalidWorkbook);
                }
                if name == b"row" && path.as_slice() == [b"worksheet".as_slice(), b"sheetData"] {
                    let actual = xml::attr(&e, b"r")?
                        .map(|s| s.parse::<u32>().map_err(|_| XlsxError::InvalidWorkbook))
                        .transpose()?
                        .unwrap_or(row + 1);
                    if actual <= row || actual > 1_048_576 {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    row = actual;
                    next_col = 1;
                }
                if name == b"c"
                    && path.as_slice() == [b"worksheet".as_slice(), b"sheetData", b"row"]
                {
                    let (r, col) = match xml::attr(&e, b"r")? {
                        Some(s) => coordinate(&s)?,
                        None => (row, next_col),
                    };
                    if r != row || col < next_col || (r, col) <= last || col > 16384 {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    next_col = col + 1;
                    last = (r, col);
                    let kind = xml::attr(&e, b"t")?.unwrap_or_default();
                    if !matches!(
                        kind.as_str(),
                        "" | "n" | "s" | "b" | "str" | "inlineStr" | "d" | "e"
                    ) {
                        return Err(XlsxError::UnsupportedCell);
                    }
                    pending = Some(Pending {
                        row: r,
                        col,
                        kind,
                        value: String::new(),
                        has_value: false,
                        formula: false,
                    });
                }
                if let Some(p) = pending.as_mut() {
                    if path.last().map(Vec::as_slice) == Some(b"c".as_slice()) {
                        start_value(p, &e)?;
                    }
                }
                path.push(name.to_vec());
                if empty {
                    finish(&mut path, &mut pending, strings, &mut cell)?;
                }
            }
            Event::End(_) => finish(&mut path, &mut pending, strings, &mut cell)?,
            Event::Text(t) if value_text(&path) => {
                if let Some(p) = pending.as_mut() {
                    xml::append(
                        &mut p.value,
                        &t.unescape().map_err(|_| XlsxError::InvalidXml)?,
                    )?;
                }
            }
            Event::CData(t) if value_text(&path) => {
                if let Some(p) = pending.as_mut() {
                    xml::append(
                        &mut p.value,
                        std::str::from_utf8(&t).map_err(|_| XlsxError::InvalidXml)?,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    if !root || !sheet_data || pending.is_some() {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok(())
}

fn start_value(p: &mut Pending, e: &BytesStart<'_>) -> Result<(), XlsxError> {
    match e.local_name().as_ref() {
        b"v" | b"is" => {
            if p.has_value || (e.local_name().as_ref() == b"is") != (p.kind == "inlineStr") {
                return Err(XlsxError::InvalidWorkbook);
            }
            p.has_value = true;
        }
        b"f" => {
            if p.formula {
                return Err(XlsxError::InvalidWorkbook);
            }
            p.formula = true;
        }
        _ => {}
    }
    Ok(())
}

fn value_text(path: &[Vec<u8>]) -> bool {
    path == [b"worksheet".as_slice(), b"sheetData", b"row", b"c", b"v"]
        || path
            == [
                b"worksheet".as_slice(),
                b"sheetData",
                b"row",
                b"c",
                b"is",
                b"t",
            ]
        || path
            == [
                b"worksheet".as_slice(),
                b"sheetData",
                b"row",
                b"c",
                b"is",
                b"r",
                b"t",
            ]
}

fn finish<'a>(
    path: &mut Vec<Vec<u8>>,
    pending: &mut Option<Pending>,
    strings: &'a [String],
    cell: &mut impl FnMut(Cell<'a>) -> Result<(), XlsxError>,
) -> Result<(), XlsxError> {
    if path.pop().as_deref() != Some(b"c".as_slice()) {
        return Ok(());
    }
    let Some(p) = pending.take() else {
        return Err(XlsxError::InvalidWorkbook);
    };
    if p.formula && !p.has_value {
        return Err(XlsxError::MissingFormulaCache);
    }
    if !p.has_value {
        return Ok(());
    }
    let value = match p.kind.as_str() {
        "s" => {
            let i = p
                .value
                .parse::<usize>()
                .map_err(|_| XlsxError::InvalidWorkbook)?;
            Cow::Borrowed(strings.get(i).ok_or(XlsxError::InvalidWorkbook)?.as_str())
        }
        "b" => match p.value.as_str() {
            "0" => Cow::Borrowed("false"),
            "1" => Cow::Borrowed("true"),
            _ => return Err(XlsxError::UnsupportedCell),
        },
        "n" | "" => {
            if !number(&p.value) {
                return Err(XlsxError::UnsupportedCell);
            }
            Cow::Owned(p.value)
        }
        "e" => Cow::Borrowed(match p.value.as_str() {
            "#DIV/0!" => "#ERR:Div0",
            "#N/A" => "#ERR:NA",
            "#NAME?" => "#ERR:Name",
            "#NULL!" => "#ERR:Null",
            "#NUM!" => "#ERR:Num",
            "#REF!" => "#ERR:Ref",
            "#VALUE!" => "#ERR:Value",
            "#GETTING_DATA" | "#DATA!" => "#ERR:GettingData",
            _ => return Err(XlsxError::UnsupportedCell),
        }),
        "inlineStr" | "str" => Cow::Owned(xml::excel_text(p.value)?),
        _ => Cow::Owned(p.value),
    };
    cell(Cell {
        row: p.row,
        col: p.col,
        value,
        formula: p.formula,
    })
}

fn coordinate(value: &str) -> Result<(u32, u32), XlsxError> {
    let mut col = 0_u32;
    let split = value.bytes().take_while(u8::is_ascii_uppercase).count();
    if split == 0 || split > 3 {
        return Err(XlsxError::InvalidWorkbook);
    }
    for b in value.bytes().take(split) {
        col = col * 26 + u32::from(b - b'A' + 1);
    }
    let digits = &value[split..];
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(XlsxError::InvalidWorkbook);
    }
    let row = digits
        .parse::<u32>()
        .map_err(|_| XlsxError::InvalidWorkbook)?;
    if row == 0 || row > 1_048_576 || col > 16384 {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok((row, col))
}

fn number(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut at = usize::from(matches!(bytes.first(), Some(b'+' | b'-')));
    let mut digits = 0;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at += 1;
        digits += 1;
    }
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return false;
    }
    if matches!(bytes.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        if at == start {
            return false;
        }
    }
    at == bytes.len()
}

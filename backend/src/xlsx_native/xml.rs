use super::{check_cancel, Limit, XlsxError, MAX_FIELD_BYTES};
use quick_xml::{
    events::{BytesStart, Event},
    name::ResolveResult,
    reader::{NsReader, Reader},
};
use std::sync::atomic::AtomicBool;

pub(super) const MAIN: &[u8] = b"http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub(super) const REL: &[u8] =
    b"http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(super) const PACKAGE: &[u8] = b"http://schemas.openxmlformats.org/package/2006/relationships";

/// First pass runs without an SST or row. Even the library's internal name-copy
/// before returning Start is bounded by the admitted XML buffer. Subsequent
/// passes may use namespace resolution after all names/attributes are bounded.
pub(super) fn validate(bytes: &[u8], cancel: &AtomicBool) -> Result<(), XlsxError> {
    let input = std::str::from_utf8(bytes).map_err(|_| XlsxError::UnsupportedXml)?;
    if !valid_characters(input) {
        return Err(XlsxError::InvalidXml);
    }
    let mut reader = Reader::from_str(input);
    let mut depth = 0_usize;
    let mut roots = 0;
    loop {
        check_cancel(cancel)?;
        let next = reader.read_event().map_err(|_| XlsxError::InvalidXml)?;
        let empty = matches!(&next, Event::Empty(_));
        match next {
            Event::Start(e) | Event::Empty(e) => {
                // Empty elements do not enter the internal element stack.
                if depth == 0 {
                    roots += 1;
                }
                if roots > 1 || depth >= 64 || e.name().as_ref().len() > 128 {
                    return Err(XlsxError::Limit(Limit::Structure));
                }
                let mut count = 0;
                for a in e.attributes() {
                    count += 1;
                    let a = a.map_err(|_| XlsxError::InvalidXml)?;
                    if count > 64 || a.key.as_ref().len() > 128 || a.value.len() > 4096 {
                        return Err(XlsxError::Limit(Limit::Structure));
                    }
                    if a.key.as_namespace_binding().is_some() && a.value.len() > 256 {
                        return Err(XlsxError::Limit(Limit::Structure));
                    }
                    unescape(&a.value)?;
                }
                if !empty {
                    depth += 1;
                }
            }
            Event::End(_) => {
                depth = depth.checked_sub(1).ok_or(XlsxError::InvalidXml)?;
            }
            Event::Text(t) => {
                if t.len() > MAX_FIELD_BYTES {
                    return Err(XlsxError::Limit(Limit::Field));
                }
                let value = t.unescape().map_err(|_| XlsxError::UnsupportedXml)?;
                if !valid_characters(&value) {
                    return Err(XlsxError::InvalidXml);
                }
                if depth == 0 && !value.trim().is_empty() {
                    return Err(XlsxError::InvalidXml);
                }
            }
            Event::CData(t) => {
                if depth == 0 {
                    return Err(XlsxError::InvalidXml);
                }
                if t.len() > MAX_FIELD_BYTES {
                    return Err(XlsxError::Limit(Limit::Field));
                }
            }
            Event::Decl(d) => {
                if let Some(encoding) = d.encoding() {
                    let encoding = encoding.map_err(|_| XlsxError::InvalidXml)?;
                    if !encoding.eq_ignore_ascii_case(b"UTF-8") {
                        return Err(XlsxError::UnsupportedXml);
                    }
                }
            }
            Event::DocType(_) => return Err(XlsxError::UnsupportedXml),
            Event::Eof => {
                return if depth == 0 && roots == 1 {
                    Ok(())
                } else {
                    Err(XlsxError::InvalidXml)
                }
            }
            _ => {}
        }
    }
}

pub(super) fn walk<'a>(
    bytes: &'a [u8],
    cancel: &AtomicBool,
    mut event: impl FnMut(&NsReader<&'a [u8]>, Event<'a>) -> Result<(), XlsxError>,
) -> Result<(), XlsxError> {
    let mut reader = NsReader::from_reader(bytes);
    loop {
        check_cancel(cancel)?;
        let next = reader.read_event().map_err(|_| XlsxError::InvalidXml)?;
        if matches!(next, Event::Eof) {
            return Ok(());
        }
        event(&reader, next)?;
    }
}

pub(super) fn in_ns(reader: &NsReader<&[u8]>, element: &BytesStart<'_>, ns: &[u8]) -> bool {
    matches!(reader.resolve_element(element.name()).0, ResolveResult::Bound(found) if found.as_ref() == ns)
}

pub(super) fn attr(element: &BytesStart<'_>, key: &[u8]) -> Result<Option<String>, XlsxError> {
    for a in element.attributes() {
        let a = a.map_err(|_| XlsxError::InvalidXml)?;
        if a.key.as_ref() == key {
            return unescape(&a.value).map(|s| Some(s.into_owned()));
        }
    }
    Ok(None)
}

pub(super) fn relationship_id(
    reader: &NsReader<&[u8]>,
    element: &BytesStart<'_>,
) -> Result<String, XlsxError> {
    for a in element.attributes() {
        let a = a.map_err(|_| XlsxError::InvalidXml)?;
        let (ns, local) = reader.resolve_attribute(a.key);
        if local.as_ref() == b"id" && matches!(ns, ResolveResult::Bound(ns) if ns.as_ref() == REL) {
            return unescape(&a.value).map(|s| s.into_owned());
        }
    }
    Err(XlsxError::InvalidWorkbook)
}

pub(super) fn unescape(value: &[u8]) -> Result<std::borrow::Cow<'_, str>, XlsxError> {
    let text = std::str::from_utf8(value).map_err(|_| XlsxError::UnsupportedXml)?;
    let value = quick_xml::escape::unescape(text).map_err(|_| XlsxError::UnsupportedXml)?;
    if !valid_characters(&value) {
        return Err(XlsxError::InvalidXml);
    }
    Ok(value)
}

fn valid_characters(value: &str) -> bool {
    !value.chars().any(|c| matches!(c, '\0'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'))
}

/// SpreadsheetML escapes are separate from XML entities. Decode once, preserving
/// escaped underscores, and pair UTF-16 surrogate escapes before creating UTF-8.
pub(super) fn excel_text(value: String) -> Result<String, XlsxError> {
    fn code(bytes: &[u8], at: usize) -> Option<u16> {
        let token = bytes.get(at..at + 7)?;
        if token[0] != b'_' || token[1] != b'x' || token[6] != b'_' {
            return None;
        }
        let digits = std::str::from_utf8(&token[2..6]).ok()?;
        if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u16::from_str_radix(digits, 16).ok()
    }
    let bytes = value.as_bytes();
    if !bytes.windows(2).any(|b| b == b"_x") {
        return Ok(value);
    }
    let mut result = String::with_capacity(value.len());
    let mut at = 0;
    while at < bytes.len() {
        if let Some(first) = code(bytes, at) {
            at += 7;
            let scalar = if (0xd800..=0xdbff).contains(&first) {
                let second = code(bytes, at)
                    .filter(|n| (0xdc00..=0xdfff).contains(n))
                    .ok_or(XlsxError::UnsupportedCell)?;
                at += 7;
                0x10000 + (u32::from(first - 0xd800) << 10) + u32::from(second - 0xdc00)
            } else {
                u32::from(first)
            };
            let character = char::from_u32(scalar).ok_or(XlsxError::UnsupportedCell)?;
            if character == '\0' {
                return Err(XlsxError::UnsupportedCell);
            }
            result.push(character);
        } else {
            let c = value[at..]
                .chars()
                .next()
                .ok_or(XlsxError::UnsupportedCell)?;
            result.push(c);
            at += c.len_utf8();
        }
    }
    Ok(result)
}

pub(super) fn append(target: &mut String, value: &str) -> Result<(), XlsxError> {
    let size = target
        .len()
        .checked_add(value.len())
        .ok_or(XlsxError::Limit(Limit::Field))?;
    if size > MAX_FIELD_BYTES {
        return Err(XlsxError::Limit(Limit::Field));
    }
    if size > target.capacity() {
        let next = size
            .max(target.capacity().saturating_mul(2))
            .min(MAX_FIELD_BYTES);
        target
            .try_reserve_exact(next - target.len())
            .map_err(|_| XlsxError::Limit(Limit::Field))?;
    }
    if target.capacity() > MAX_FIELD_BYTES {
        return Err(XlsxError::Limit(Limit::Field));
    }
    target.push_str(value);
    Ok(())
}

use super::{xml, Limit, XlsxError, MAX_STRINGS, MAX_STRINGS_BYTES};
use quick_xml::events::Event;
use std::{mem::size_of, sync::atomic::AtomicBool};

pub(super) fn read(bytes: &[u8], cancel: &AtomicBool) -> Result<Vec<String>, XlsxError> {
    // One bounded allocation avoids quadratic reallocations for many short strings.
    let mut result = Vec::with_capacity(MAX_STRINGS);
    let mut path: Vec<Vec<u8>> = Vec::new();
    let mut current = String::new();
    let mut retained = 0_usize;
    let mut root = false;
    xml::walk(bytes, cancel, |reader, event| {
        let empty = matches!(&event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                if !xml::in_ns(reader, &e, xml::MAIN) {
                    return Err(XlsxError::InvalidWorkbook);
                }
                let name = e.local_name().into_inner();
                let allowed = match path.last().map(Vec::as_slice) {
                    Some(b"sst") => name == b"si",
                    Some(b"si") => matches!(name, b"t" | b"r" | b"rPh" | b"phoneticPr"),
                    Some(b"r") => matches!(name, b"t" | b"rPr"),
                    Some(b"t") => false,
                    _ => true,
                };
                if !allowed {
                    return Err(XlsxError::UnsupportedCell);
                }
                if path.is_empty() {
                    if name != b"sst" || root {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    root = true;
                }
                if name == b"si" && path.as_slice() != [b"sst".as_slice()] {
                    return Err(XlsxError::InvalidWorkbook);
                }
                if name == b"si" {
                    current = String::new();
                }
                path.push(name.to_vec());
                if empty {
                    finish(&mut path, &mut current, &mut result, &mut retained)?;
                }
            }
            Event::End(_) => finish(&mut path, &mut current, &mut result, &mut retained)?,
            Event::Text(t) if is_text(&path) => xml::append(
                &mut current,
                &t.unescape().map_err(|_| XlsxError::InvalidXml)?,
            )?,
            Event::CData(t) if is_text(&path) => xml::append(
                &mut current,
                std::str::from_utf8(&t).map_err(|_| XlsxError::InvalidXml)?,
            )?,
            _ => {}
        }
        Ok(())
    })?;
    if !root {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok(result)
}

fn is_text(path: &[Vec<u8>]) -> bool {
    path == [b"sst".as_slice(), b"si", b"t"] || path == [b"sst".as_slice(), b"si", b"r", b"t"]
}

fn finish(
    path: &mut Vec<Vec<u8>>,
    current: &mut String,
    result: &mut Vec<String>,
    retained: &mut usize,
) -> Result<(), XlsxError> {
    if path.pop().as_deref() == Some(b"si".as_slice()) {
        *current = xml::excel_text(std::mem::take(current))?;
        if result.len() >= MAX_STRINGS {
            return Err(XlsxError::Limit(Limit::SharedStrings));
        }
        let strings = retained
            .checked_add(current.capacity())
            .ok_or(XlsxError::Limit(Limit::SharedStrings))?;
        let needed = (result.len() + 1)
            .checked_mul(size_of::<String>())
            .and_then(|n| n.checked_add(strings))
            .ok_or(XlsxError::Limit(Limit::SharedStrings))?;
        if needed > MAX_STRINGS_BYTES {
            return Err(XlsxError::Limit(Limit::SharedStrings));
        }
        result
            .try_reserve_exact(1)
            .map_err(|_| XlsxError::Limit(Limit::SharedStrings))?;
        if result.capacity() * size_of::<String>() + strings > MAX_STRINGS_BYTES {
            return Err(XlsxError::Limit(Limit::SharedStrings));
        }
        result.push(std::mem::take(current));
        *retained = strings;
    }
    Ok(())
}

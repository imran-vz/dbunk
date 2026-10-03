use super::{
    archive, xml, Limit, SheetId, SheetInfo, SheetVisibility, Workbook, XlsxError,
    MAX_METADATA_BYTES, MAX_SHEETS,
};
use quick_xml::events::Event;
use std::{
    io::{Read, Seek},
    sync::atomic::AtomicBool,
};

struct Relationship {
    id: String,
    kind: String,
    target: String,
    external: bool,
}

fn relationships<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
    cancel: &AtomicBool,
) -> Result<Vec<Relationship>, XlsxError> {
    let bytes = archive::part(archive, path, MAX_METADATA_BYTES, cancel)?;
    xml::validate(&bytes, cancel)?;
    let mut result = Vec::new();
    let mut root = false;
    let mut depth = 0;
    xml::walk(&bytes, cancel, |reader, event| {
        let empty = matches!(&event, Event::Empty(_));
        let end = matches!(&event, Event::End(_));
        if let Event::Start(e) | Event::Empty(e) = event {
            if !xml::in_ns(reader, &e, xml::PACKAGE) {
                return Err(XlsxError::InvalidWorkbook);
            }
            match e.local_name().as_ref() {
                b"Relationships" if depth == 0 && !root => root = true,
                b"Relationship" if depth == 1 && root => {
                    if result.len() >= 2048 {
                        return Err(XlsxError::Limit(Limit::Archive));
                    }
                    let id = xml::attr(&e, b"Id")?.ok_or(XlsxError::InvalidWorkbook)?;
                    let kind = xml::attr(&e, b"Type")?.ok_or(XlsxError::InvalidWorkbook)?;
                    let target = xml::attr(&e, b"Target")?.ok_or(XlsxError::InvalidWorkbook)?;
                    if id.len() > 256
                        || kind.len() > 256
                        || target.len() > 1024
                        || result.iter().any(|r: &Relationship| r.id == id)
                    {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    let external = match xml::attr(&e, b"TargetMode")?.as_deref() {
                        None | Some("Internal") => false,
                        Some("External") => true,
                        _ => return Err(XlsxError::InvalidWorkbook),
                    };
                    result.push(Relationship {
                        id,
                        kind,
                        target,
                        external,
                    });
                }
                _ => return Err(XlsxError::InvalidWorkbook),
            }
            if !empty {
                depth += 1;
            }
        } else if end {
            depth -= 1;
        }
        Ok(())
    })?;
    if !root {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok(result)
}

fn target(base: &str, r: &Relationship) -> Result<String, XlsxError> {
    if r.external {
        return Err(XlsxError::InvalidWorkbook);
    }
    let path = if let Some(absolute) = r.target.strip_prefix('/') {
        absolute.to_owned()
    } else if let Some((parent, _)) = base.rsplit_once('/') {
        format!("{parent}/{}", r.target)
    } else {
        r.target.clone()
    };
    if path.len() > 1024 || !archive::valid_path(&path) {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok(path)
}

pub(super) fn load<R: Read + Seek>(
    workbook: &mut Workbook<R>,
    cancel: &AtomicBool,
) -> Result<(), XlsxError> {
    let root = relationships(&mut workbook.archive, "_rels/.rels", cancel)?;
    let mut office = root.iter().filter(|r| r.kind == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument");
    let workbook_path = target("", office.next().ok_or(XlsxError::InvalidWorkbook)?)?;
    if office.next().is_some() {
        return Err(XlsxError::InvalidWorkbook);
    }
    drop(root);
    let relationship_path = match workbook_path.rsplit_once('/') {
        Some((parent, name)) => format!("{parent}/_rels/{name}.rels"),
        None => format!("_rels/{workbook_path}.rels"),
    };
    let rels = relationships(&mut workbook.archive, &relationship_path, cancel)?;
    let mut strings = rels.iter().filter(|r| r.kind == "http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings");
    workbook.strings_path = strings
        .next()
        .map(|r| target(&workbook_path, r))
        .transpose()?;
    if strings.next().is_some() {
        return Err(XlsxError::InvalidWorkbook);
    }
    let bytes = archive::part(
        &mut workbook.archive,
        &workbook_path,
        MAX_METADATA_BYTES,
        cancel,
    )?;
    xml::validate(&bytes, cancel)?;
    let mut root_seen = false;
    let mut sheets_seen = false;
    let mut path: Vec<Vec<u8>> = Vec::new();
    xml::walk(&bytes, cancel, |reader, event| {
        let empty = matches!(&event, Event::Empty(_));
        let end = matches!(&event, Event::End(_));
        if let Event::Start(e) | Event::Empty(e) = event {
            if !xml::in_ns(reader, &e, xml::MAIN) {
                return Err(XlsxError::InvalidWorkbook);
            }
            let tag = e.local_name();
            if path.is_empty() && tag.as_ref() != b"workbook" {
                return Err(XlsxError::InvalidWorkbook);
            }
            match tag.as_ref() {
                b"workbook" if path.is_empty() && !root_seen => root_seen = true,
                b"sheets" if path.as_slice() == [b"workbook".as_slice()] && !sheets_seen => {
                    sheets_seen = true
                }
                b"sheet" if path.as_slice() == [b"workbook".as_slice(), b"sheets"] => {
                    if workbook.sheets.len() >= MAX_SHEETS {
                        return Err(XlsxError::Limit(Limit::Sheets));
                    }
                    let name = xml::excel_text(
                        xml::attr(&e, b"name")?.ok_or(XlsxError::InvalidWorkbook)?,
                    )?;
                    if name.is_empty()
                        || name.len() > 256
                        || workbook.sheets.iter().any(|s| s.name == name)
                    {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    let id = xml::relationship_id(reader, &e)?;
                    let r = rels
                        .iter()
                        .find(|r| r.id == id)
                        .ok_or(XlsxError::InvalidWorkbook)?;
                    if r.kind != "http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    let part_path = target(&workbook_path, r)?;
                    if workbook.paths.contains(&part_path) {
                        return Err(XlsxError::InvalidWorkbook);
                    }
                    let visibility = match xml::attr(&e, b"state")?.as_deref() {
                        None | Some("visible") => SheetVisibility::Visible,
                        Some("hidden") => SheetVisibility::Hidden,
                        Some("veryHidden") => SheetVisibility::VeryHidden,
                        _ => return Err(XlsxError::InvalidWorkbook),
                    };
                    workbook.sheets.push(SheetInfo {
                        id: SheetId(workbook.sheets.len() as u16),
                        name,
                        visibility,
                    });
                    workbook.paths.push(part_path);
                }
                b"workbook" | b"sheets" | b"sheet" => return Err(XlsxError::InvalidWorkbook),
                _ => {}
            }
            if !empty {
                path.push(tag.as_ref().to_vec());
            }
        } else if end {
            path.pop();
        }
        Ok(())
    })?;
    if !root_seen || workbook.sheets.is_empty() {
        return Err(XlsxError::InvalidWorkbook);
    }
    Ok(())
}

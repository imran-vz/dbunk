use super::{check_cancel, Limit, XlsxError};
use std::{
    collections::HashSet,
    io::{Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};

const MAX_ENTRIES: usize = 2_048;
const MAX_DIRECTORY: usize = 512 * 1024;
const MAX_NAME: usize = 1024;
fn u16_at(b: &[u8], n: usize) -> u16 {
    u16::from_le_bytes([b[n], b[n + 1]])
}
fn u32_at(b: &[u8], n: usize) -> u32 {
    u32::from_le_bytes([b[n], b[n + 1], b[n + 2], b[n + 3]])
}
fn read(source: &mut impl Read, b: &mut [u8]) -> Result<(), XlsxError> {
    source.read_exact(b).map_err(|_| XlsxError::InputIo)
}
fn seek(source: &mut impl Seek, n: u64) -> Result<(), XlsxError> {
    source
        .seek(SeekFrom::Start(n))
        .map(|_| ())
        .map_err(|_| XlsxError::InputIo)
}

/// ZipArchive::new has no allocation cap and retries older EOCD records. Refuse
/// every ambiguous signature before allowing that parser to allocate metadata.
pub(super) fn preflight<R: Read + Seek>(
    source: &mut R,
    cancel: &AtomicBool,
) -> Result<(), XlsxError> {
    check_cancel(cancel)?;
    let length = source
        .seek(SeekFrom::End(0))
        .map_err(|_| XlsxError::InputIo)?;
    seek(source, 0)?;
    let mut buffer = [0_u8; 65536];
    let mut rolling = 0_u32;
    let mut position = 0_u64;
    let mut end = None;
    while position < length {
        check_cancel(cancel)?;
        let count = usize::try_from((length - position).min(buffer.len() as u64)).unwrap();
        read(source, &mut buffer[..count])?;
        for &byte in &buffer[..count] {
            rolling = (rolling << 8) | u32::from(byte);
            position += 1;
            if rolling == 0x504b0506 && end.replace(position - 4).is_some() {
                return Err(XlsxError::AmbiguousZip);
            }
        }
    }
    let end = end.ok_or(XlsxError::InvalidZip)?;
    seek(source, end)?;
    let mut eocd = [0; 22];
    read(source, &mut eocd)?;
    let count = usize::from(u16_at(&eocd, 10));
    let size = u32_at(&eocd, 12);
    let offset = u32_at(&eocd, 16);
    if u16_at(&eocd, 4) != 0
        || u16_at(&eocd, 6) != 0
        || usize::from(u16_at(&eocd, 8)) != count
        || count == 65535
        || size == u32::MAX
        || offset == u32::MAX
    {
        return Err(XlsxError::UnsupportedZip);
    }
    if count == 0
        || count > MAX_ENTRIES
        || size as usize > MAX_DIRECTORY
        || usize::from(u16_at(&eocd, 20)) > MAX_NAME
    {
        return Err(XlsxError::Limit(Limit::Archive));
    }
    if end.checked_add(22 + u64::from(u16_at(&eocd, 20))) != Some(length)
        || u64::from(offset).checked_add(u64::from(size)) != Some(end)
    {
        return Err(XlsxError::InvalidZip);
    }
    seek(source, u64::from(offset))?;
    let mut directory = vec![0; size as usize];
    read(source, &mut directory)?;
    let mut names = HashSet::with_capacity(count);
    let mut at = 0;
    for _ in 0..count {
        check_cancel(cancel)?;
        let header = directory.get(at..at + 46).ok_or(XlsxError::InvalidZip)?;
        if &header[..4] != b"PK\x01\x02" {
            return Err(XlsxError::InvalidZip);
        }
        let flags = u16_at(header, 8);
        let method = u16_at(header, 10);
        if flags & !0x080e != 0
            || u16_at(header, 6) > 20
            || !matches!(method, 0 | 8)
            || u16_at(header, 34) != 0
            || u32_at(header, 20) == u32::MAX
            || u32_at(header, 24) == u32::MAX
            || u32_at(header, 42) == u32::MAX
        {
            return Err(XlsxError::UnsupportedZip);
        }
        let name_len = usize::from(u16_at(header, 28));
        let extra_len = usize::from(u16_at(header, 30));
        let comment_len = usize::from(u16_at(header, 32));
        if name_len == 0
            || [name_len, extra_len, comment_len]
                .iter()
                .any(|n| *n > MAX_NAME)
        {
            return Err(XlsxError::Limit(Limit::Archive));
        }
        let next = at + 46 + name_len + extra_len + comment_len;
        let variable = directory.get(at + 46..next).ok_or(XlsxError::InvalidZip)?;
        let name = &variable[..name_len];
        let decoded = std::str::from_utf8(name).map_err(|_| XlsxError::UnsupportedZip)?;
        if (flags & 0x800 == 0 && !name.is_ascii()) || !valid_path(decoded) || !names.insert(name) {
            return Err(XlsxError::InvalidZip);
        }
        extras(&variable[name_len..name_len + extra_len])?;
        // Local ZIP64/encryption cannot hide behind classic central metadata.
        let local_offset = u64::from(u32_at(header, 42));
        if local_offset + 30 > u64::from(offset) {
            return Err(XlsxError::InvalidZip);
        }
        seek(source, local_offset)?;
        let mut local = [0; 30];
        read(source, &mut local)?;
        if &local[..4] != b"PK\x03\x04"
            || u16_at(&local, 4) > 20
            || u16_at(&local, 6) != flags
            || u16_at(&local, 8) != method
            || u32_at(&local, 18) == u32::MAX
            || u32_at(&local, 22) == u32::MAX
        {
            return Err(XlsxError::UnsupportedZip);
        }
        let local_name = usize::from(u16_at(&local, 26));
        let local_extra = usize::from(u16_at(&local, 28));
        if local_name != name_len || local_extra > MAX_NAME {
            return Err(XlsxError::Limit(Limit::Archive));
        }
        let mut local_fields = [0; MAX_NAME * 2];
        read(source, &mut local_fields[..local_name + local_extra])?;
        if &local_fields[..local_name] != name {
            return Err(XlsxError::InvalidZip);
        }
        extras(&local_fields[local_name..local_name + local_extra])?;
        if local_offset + 30 + (local_name + local_extra) as u64 + u64::from(u32_at(header, 20))
            > u64::from(offset)
        {
            return Err(XlsxError::InvalidZip);
        }
        at = next;
    }
    if at != directory.len() {
        return Err(XlsxError::InvalidZip);
    }
    seek(source, 0)
}

fn extras(extra: &[u8]) -> Result<(), XlsxError> {
    let mut e = 0;
    while e < extra.len() {
        let h = extra.get(e..e + 4).ok_or(XlsxError::InvalidZip)?;
        // Unicode Path replaces the already checked raw filename inside zip's
        // metadata parser. Refuse that alternate identity rather than validating
        // one name and admitting another (including a duplicate).
        if matches!(u16_at(h, 0), 1 | 0x9901 | 0x7075) {
            return Err(XlsxError::UnsupportedZip);
        }
        e = e
            .checked_add(4 + usize::from(u16_at(h, 2)))
            .ok_or(XlsxError::InvalidZip)?;
        if e > extra.len() {
            return Err(XlsxError::InvalidZip);
        }
    }
    Ok(())
}

pub(super) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0', ':', '%', '#', '?'])
        && path.split('/').all(|p| !matches!(p, "." | ".."))
}

pub(super) fn part<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    path: &str,
    max: usize,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, XlsxError> {
    check_cancel(cancel)?;
    let mut entry = archive
        .by_name(path)
        .map_err(|_| XlsxError::InvalidWorkbook)?;
    let size = usize::try_from(entry.size()).map_err(|_| XlsxError::Limit(Limit::Xml))?;
    if size > max {
        return Err(XlsxError::Limit(Limit::Xml));
    }
    let mut bytes = vec![0; size];
    for chunk in bytes.chunks_mut(65536) {
        check_cancel(cancel)?;
        read(&mut entry, chunk)?;
    }
    let mut extra = [0];
    if entry.read(&mut extra).map_err(|_| XlsxError::InvalidZip)? != 0 {
        return Err(XlsxError::InvalidZip);
    }
    Ok(bytes)
}

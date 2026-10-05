//! Strict data-only PE resource reader. Offsets stay inside retained file/raw
//! section bounds. Ambiguous duplicate version leaves are refused.
use super::*;
use std::io::{Read, Seek, SeekFrom};
fn invalid<T>() -> capture::Result<T> {
    Err(capture::Error::Invalid)
}
fn range(bytes: &[u8], at: usize, len: usize) -> capture::Result<&[u8]> {
    bytes
        .get(at..at.checked_add(len).ok_or(capture::Error::Limit)?)
        .ok_or(capture::Error::Invalid)
}
fn u16le(bytes: &[u8], at: usize) -> capture::Result<u16> {
    Ok(u16::from_le_bytes(
        range(bytes, at, 2)?
            .try_into()
            .map_err(|_| capture::Error::Invalid)?,
    ))
}
fn u32le(bytes: &[u8], at: usize) -> capture::Result<u32> {
    Ok(u32::from_le_bytes(
        range(bytes, at, 4)?
            .try_into()
            .map_err(|_| capture::Error::Invalid)?,
    ))
}
fn read<R: Read + Seek>(file: &mut R, size: u64, at: u64, len: usize) -> capture::Result<Vec<u8>> {
    if len > 4 * 1024 * 1024 || at > size || len as u64 > size - at {
        return Err(capture::Error::Limit);
    }
    file.seek(SeekFrom::Start(at))
        .map_err(|_| capture::Error::Platform)?;
    let mut result = vec![0; len];
    file.read_exact(&mut result)
        .map_err(|_| capture::Error::Platform)?;
    Ok(result)
}
fn entries(bytes: &[u8], at: usize) -> capture::Result<Vec<(u32, u32)>> {
    let names = usize::from(u16le(bytes, at + 12)?);
    let ids = usize::from(u16le(bytes, at + 14)?);
    let count = names.checked_add(ids).ok_or(capture::Error::Limit)?;
    if count > 4096 {
        return Err(capture::Error::Limit);
    }
    range(bytes, at + 16, count * 8)?;
    (0..count)
        .map(|i| {
            Ok((
                u32le(bytes, at + 16 + i * 8)?,
                u32le(bytes, at + 20 + i * 8)?,
            ))
        })
        .collect()
}
fn directory(bytes: &[u8], value: u32) -> capture::Result<Vec<(u32, u32)>> {
    if value & 0x80000000 == 0 {
        return invalid();
    }
    entries(bytes, (value & 0x7fffffff) as usize)
}
fn fixed_version(data: &[u8]) -> capture::Result<RemoteEditorFileVersion> {
    let length = usize::from(u16le(data, 0)?);
    if length > data.len() || length < 92 || u16le(data, 2)? != 52 || u16le(data, 4)? != 0 {
        return invalid();
    }
    let key = "VS_VERSION_INFO"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    for (i, value) in key.iter().enumerate() {
        if u16le(data, 6 + i * 2)? != *value {
            return invalid();
        }
    }
    let at = (6 + key.len() * 2 + 3) & !3;
    range(data, at, 52)?;
    if at + 52 > length || u32le(data, at)? != 0xfeef04bd || u32le(data, at + 4)? != 0x00010000 {
        return invalid();
    }
    let ms = u32le(data, at + 8)?;
    let ls = u32le(data, at + 12)?;
    Ok(RemoteEditorFileVersion {
        major: (ms >> 16) as u16,
        minor: ms as u16,
        build: (ls >> 16) as u16,
        revision: ls as u16,
    })
}
pub(super) fn file_version<R: Read + Seek>(
    file: &mut R,
    size: u64,
    check: &impl Fn() -> capture::Result<()>,
) -> capture::Result<Option<RemoteEditorFileVersion>> {
    let dos = read(file, size, 0, 64)?;
    if range(&dos, 0, 2)? != b"MZ" {
        return invalid();
    }
    let pe_at = u64::from(u32le(&dos, 60)?);
    if !(64..=1024 * 1024).contains(&pe_at) {
        return Err(capture::Error::Limit);
    }
    let header = read(file, size, pe_at, 24)?;
    if range(&header, 0, 4)? != b"PE\0\0" {
        return invalid();
    }
    let sections = usize::from(u16le(&header, 6)?);
    let optional = usize::from(u16le(&header, 20)?);
    if sections == 0 || sections > 96 || !(112..=4096).contains(&optional) {
        return Err(capture::Error::Limit);
    }
    let opts = read(file, size, pe_at + 24, optional)?;
    let dirs = match u16le(&opts, 0)? {
        0x10b => 96,
        0x20b => 112,
        _ => return invalid(),
    };
    if u32le(&opts, dirs - 4)? < 3 {
        return Ok(None);
    }
    let rva = u32le(&opts, dirs + 16)?;
    let resource_size = u32le(&opts, dirs + 20)?;
    if rva == 0 && resource_size == 0 {
        return Ok(None);
    }
    if rva == 0 || !(16..=4 * 1024 * 1024).contains(&resource_size) {
        return Err(capture::Error::Limit);
    }
    let table = read(file, size, pe_at + 24 + optional as u64, sections * 40)?;
    let mut section = None;
    for i in 0..sections {
        check()?;
        let at = i * 40;
        let address = u32le(&table, at + 12)?;
        let raw = u32le(&table, at + 16)?;
        let offset = u32le(&table, at + 20)?;
        if rva >= address && u64::from(rva - address) + u64::from(resource_size) <= u64::from(raw) {
            if section.is_some() {
                return invalid();
            }
            section = Some((address, raw, offset));
        }
    }
    let (address, raw, offset) = section.ok_or(capture::Error::Invalid)?;
    if u64::from(offset) + u64::from(raw) > size {
        return invalid();
    }
    let resources = read(
        file,
        size,
        u64::from(offset) + u64::from(rva - address),
        resource_size as usize,
    )?;
    let version = entries(&resources, 0)?
        .into_iter()
        .filter(|(name, _)| *name == 16)
        .collect::<Vec<_>>();
    if version.is_empty() {
        return Ok(None);
    }
    if version.len() != 1 {
        return invalid();
    }
    let mut found = None;
    let mut leaves = 0usize;
    for (_, name) in directory(&resources, version[0].1)? {
        check()?;
        for (_, language) in directory(&resources, name)? {
            check()?;
            leaves += 1;
            if leaves > 32 || language & 0x80000000 != 0 {
                return Err(capture::Error::Limit);
            }
            let data_at = language as usize;
            let data_rva = u32le(&resources, data_at)?;
            let length = u32le(&resources, data_at + 4)?;
            if u32le(&resources, data_at + 12)? != 0
                || !(92..=65536).contains(&length)
                || data_rva < address
                || u64::from(data_rva - address) + u64::from(length) > u64::from(raw)
            {
                return invalid();
            }
            let data = read(
                file,
                size,
                u64::from(offset) + u64::from(data_rva - address),
                length as usize,
            )?;
            let current = fixed_version(&data)?;
            if found.is_some_and(|v| v != current) {
                return invalid();
            }
            found = Some(current);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn w16(bytes: &mut [u8], at: usize, v: u16) {
        bytes[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn w32(bytes: &mut [u8], at: usize, v: u32) {
        bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn leaf(bytes: &mut [u8], at: usize, version: [u16; 4]) {
        w16(bytes, at, 92);
        w16(bytes, at + 2, 52);
        w16(bytes, at + 4, 0);
        for (i, c) in "VS_VERSION_INFO"
            .encode_utf16()
            .chain(std::iter::once(0))
            .enumerate()
        {
            w16(bytes, at + 6 + i * 2, c);
        }
        w32(bytes, at + 40, 0xfeef04bd);
        w32(bytes, at + 44, 0x10000);
        w32(
            bytes,
            at + 48,
            (u32::from(version[0]) << 16) | u32::from(version[1]),
        );
        w32(
            bytes,
            at + 52,
            (u32::from(version[2]) << 16) | u32::from(version[3]),
        );
    }
    fn fixture(second: Option<[u16; 4]>) -> Vec<u8> {
        let mut b = vec![0u8; 1024];
        b[..2].copy_from_slice(b"MZ");
        w32(&mut b, 60, 128);
        b[128..132].copy_from_slice(b"PE\0\0");
        w16(&mut b, 134, 1);
        w16(&mut b, 148, 240);
        w16(&mut b, 152, 0x20b);
        w32(&mut b, 260, 16);
        w32(&mut b, 280, 0x1000);
        w32(&mut b, 284, 512);
        w32(&mut b, 392 + 12, 0x1000);
        w32(&mut b, 392 + 16, 512);
        w32(&mut b, 392 + 20, 512);
        w16(&mut b, 512 + 14, 1);
        w32(&mut b, 512 + 16, 16);
        w32(&mut b, 512 + 20, 0x80000020);
        w16(&mut b, 512 + 32 + 14, 1);
        w32(&mut b, 512 + 48, 1);
        w32(&mut b, 512 + 52, 0x80000040);
        w16(&mut b, 512 + 64 + 14, if second.is_some() { 2 } else { 1 });
        w32(&mut b, 512 + 80, 1033);
        w32(&mut b, 512 + 84, 128);
        w32(&mut b, 512 + 128, 0x1100);
        w32(&mut b, 512 + 132, 92);
        leaf(&mut b, 768, [5, 3, 4, 0]);
        if let Some(v) = second {
            w32(&mut b, 512 + 88, 1041);
            w32(&mut b, 512 + 92, 144);
            w32(&mut b, 512 + 144, 0x1160);
            w32(&mut b, 512 + 148, 92);
            leaf(&mut b, 864, v);
        }
        b
    }
    fn parse(b: Vec<u8>) -> capture::Result<Option<RemoteEditorFileVersion>> {
        let size = b.len() as u64;
        file_version(&mut Cursor::new(b), size, &|| Ok(()))
    }
    #[test]
    fn reads_fixed_version_from_retained_file_bytes() -> capture::Result<()> {
        assert_eq!(
            parse(fixture(None))?,
            Some(RemoteEditorFileVersion {
                major: 5,
                minor: 3,
                build: 4,
                revision: 0
            })
        );
        Ok(())
    }
    #[test]
    fn accepts_equal_language_leaves() {
        assert!(parse(fixture(Some([5, 3, 4, 0]))).is_ok());
    }
    #[test]
    fn refuses_ambiguous_language_versions() {
        assert!(parse(fixture(Some([5, 3, 5, 0]))).is_err());
    }
    #[test]
    fn refuses_nested_language_directory() {
        let mut b = fixture(None);
        w32(&mut b, 512 + 84, 0x80000040);
        assert!(parse(b).is_err());
    }
    #[test]
    fn refuses_data_outside_raw_section() {
        let mut b = fixture(None);
        w32(&mut b, 512 + 128, 0x11f0);
        assert!(parse(b).is_err());
    }
    #[test]
    fn refuses_resource_directory_census_overflow() {
        let mut b = fixture(None);
        w16(&mut b, 512 + 14, 4097);
        assert!(parse(b).is_err());
    }
    #[test]
    fn refuses_wrong_root_key() {
        let mut b = fixture(None);
        b[768 + 6] = b'X';
        assert!(parse(b).is_err());
    }
    #[test]
    fn checks_cancellation_before_resource_walk() {
        let b = fixture(None);
        let size = b.len() as u64;
        assert!(matches!(
            file_version(&mut Cursor::new(b), size, &|| Err(
                capture::Error::Cancelled
            )),
            Err(capture::Error::Cancelled)
        ));
    }
    #[test]
    fn absent_version_is_distinct_from_malformed_version() -> capture::Result<()> {
        let mut b = fixture(None);
        w32(&mut b, 280, 0);
        w32(&mut b, 284, 0);
        assert_eq!(parse(b)?, None);
        Ok(())
    }
}

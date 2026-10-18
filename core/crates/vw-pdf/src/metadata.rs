//! Bounded metadata for completed private copies, never original passthrough.
//! Replace a complete classic xref/trailer; do not append incremental history.
//! Unknown trailer keys, xref streams, encryption and /Prev are refused.
use crate::{PdfCancellation, PdfError, PdfLimits, PdfResult};

pub const MAX_EXPORT_METADATA_BYTES: usize = 16 * 1024;
const MAX_XREF_BYTES: usize = 4 * 1024 * 1024;
const MAX_TRAILER_BYTES: usize = 4096;
const MAX_ENTRIES: u32 = 200_000;
type Reference = (u32, u16);

pub(crate) fn metadata_valid(metadata: &str) -> PdfResult<()> {
    if metadata.len() > MAX_EXPORT_METADATA_BYTES {
        return Err(PdfError::Limit);
    }
    if metadata.is_empty() || !metadata.is_ascii() || metadata.as_bytes().contains(&0) {
        return Err(PdfError::Invalid);
    }
    Ok(())
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    fn token(&mut self) -> PdfResult<&'a [u8]> {
        loop {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
            if self.bytes.get(self.at) != Some(&b'%') {
                break;
            }
            while self
                .bytes
                .get(self.at)
                .is_some_and(|v| !matches!(v, b'\r' | b'\n'))
            {
                self.at += 1;
            }
        }
        let start = self.at;
        let first = *self.bytes.get(start).ok_or(PdfError::Malformed)?;
        self.at += 1;
        match first {
            b'<' if self.bytes.get(self.at) == Some(&b'<') => self.at += 1,
            b'>' if self.bytes.get(self.at) == Some(&b'>') => self.at += 1,
            b'<' => {
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|v| v.is_ascii_hexdigit() || v.is_ascii_whitespace())
                {
                    self.at += 1;
                }
                if self.bytes.get(self.at) != Some(&b'>') {
                    return Err(PdfError::Malformed);
                }
                self.at += 1;
            }
            b'[' | b']' => {}
            b'(' | b')' | b'>' | b'{' | b'}' => return Err(PdfError::Malformed),
            _ => {
                while self
                    .bytes
                    .get(self.at)
                    .is_some_and(|v| !v.is_ascii_whitespace() && !b"()<>[]{}/%".contains(v))
                {
                    self.at += 1;
                }
            }
        }
        Ok(&self.bytes[start..self.at])
    }
    fn number(&mut self) -> PdfResult<u32> {
        let token = self.token()?;
        if token.is_empty() || !token.iter().all(u8::is_ascii_digit) {
            return Err(PdfError::Malformed);
        }
        token.iter().try_fold(0u32, |n, v| {
            n.checked_mul(10)
                .and_then(|n| n.checked_add(u32::from(v - b'0')))
                .ok_or(PdfError::Malformed)
        })
    }
    fn expect(&mut self, token: &[u8]) -> PdfResult<()> {
        if self.token()? != token {
            return Err(PdfError::Malformed);
        }
        Ok(())
    }
    fn reference(&mut self) -> PdfResult<Reference> {
        let object = self.number()?;
        let generation = u16::try_from(self.number()?).map_err(|_| PdfError::Malformed)?;
        self.expect(b"R")?;
        if object == 0 {
            return Err(PdfError::Malformed);
        }
        Ok((object, generation))
    }
}
struct CrossRef<'a> {
    at: usize,
    table: &'a [u8],
    size: u32,
    root: Reference,
    info: Option<Reference>,
    id: Option<&'a [u8]>,
}

fn table_end(
    cursor: &mut Cursor<'_>,
    cancel: &PdfCancellation,
    mut visit: impl FnMut(u32, u16, u32, bool) -> PdfResult<()>,
) -> PdfResult<u32> {
    cursor.expect(b"xref")?;
    let mut previous_end = 0u32;
    let mut entries = 0u32;
    loop {
        let before = cursor.at;
        if cursor.token()? == b"trailer" {
            cursor.at = before;
            return Ok(previous_end);
        }
        cursor.at = before;
        let first = cursor.number()?;
        let count = cursor.number()?;
        let end = first.checked_add(count).ok_or(PdfError::Malformed)?;
        entries = entries.checked_add(count).ok_or(PdfError::Limit)?;
        if first < previous_end || count == 0 || entries > MAX_ENTRIES {
            return Err(PdfError::Malformed);
        }
        previous_end = end;
        for index in first..end {
            if index % 256 == 0 {
                cancel.check()?;
            }
            let offset = cursor.number()?;
            let generation = u16::try_from(cursor.number()?).map_err(|_| PdfError::Malformed)?;
            let live = match cursor.token()? {
                b"n" => true,
                b"f" => false,
                _ => return Err(PdfError::Malformed),
            };
            visit(index, generation, offset, live)?;
            if cursor.at > MAX_XREF_BYTES {
                return Err(PdfError::Limit);
            }
        }
    }
}

fn cross_ref<'a>(bytes: &'a [u8], limit: u64, cancel: &PdfCancellation) -> PdfResult<CrossRef<'a>> {
    cancel.check()?;
    if bytes.len() as u64 > limit {
        return Err(PdfError::Limit);
    }
    if !bytes.starts_with(b"%PDF-") {
        return Err(PdfError::Malformed);
    }
    let end = bytes
        .trim_ascii_end()
        .strip_suffix(b"%%EOF")
        .ok_or(PdfError::Malformed)?
        .trim_ascii_end();
    let tail_start = end.len().saturating_sub(128);
    let start = end[tail_start..]
        .windows(9)
        .rposition(|w| w == b"startxref")
        .map(|n| tail_start + n)
        .ok_or(PdfError::Malformed)?;
    let mut offset = Cursor {
        bytes: &end[start + 9..],
        at: 0,
    };
    let at = offset.number()? as usize;
    if !offset.bytes[offset.at..]
        .iter()
        .all(u8::is_ascii_whitespace)
        || at >= start
    {
        return Err(PdfError::Malformed);
    }
    let region = bytes.get(at..start).ok_or(PdfError::Malformed)?;
    if region.len() > MAX_XREF_BYTES + MAX_TRAILER_BYTES {
        return Err(PdfError::Limit);
    }
    let mut cursor = Cursor {
        bytes: region,
        at: 0,
    };
    let highest = table_end(&mut cursor, cancel, |_, _, offset, live| {
        if live && offset as usize >= at {
            return Err(PdfError::Malformed);
        }
        Ok(())
    })?;
    let table = &region[..cursor.at];
    cursor.expect(b"trailer")?;
    let trailer_start = cursor.at;
    cursor.expect(b"<<")?;
    let (mut size, mut root, mut info, mut id) = (None, None, None, None);
    loop {
        match cursor.token()? {
            b">>" => break,
            b"/Size" if size.is_none() => size = Some(cursor.number()?),
            b"/Root" if root.is_none() => root = Some(cursor.reference()?),
            b"/Info" if info.is_none() => info = Some(cursor.reference()?),
            b"/ID" if id.is_none() => {
                let begin = cursor.at;
                cursor.expect(b"[")?;
                for _ in 0..2 {
                    let value = cursor.token()?;
                    if !value.starts_with(b"<") || value.starts_with(b"<<") || value.len() > 258 {
                        return Err(PdfError::Malformed);
                    }
                }
                cursor.expect(b"]")?;
                id = Some(&region[begin..cursor.at]);
            }
            _ => return Err(PdfError::Malformed),
        }
        if cursor.at - trailer_start > MAX_TRAILER_BYTES {
            return Err(PdfError::Limit);
        }
    }
    if !region[cursor.at..].iter().all(u8::is_ascii_whitespace) {
        return Err(PdfError::Malformed);
    }
    let size = size
        .filter(|v| *v >= highest && *v > 1)
        .ok_or(PdfError::Malformed)?;
    let root = root.filter(|v| v.0 < size).ok_or(PdfError::Malformed)?;
    if info.is_some_and(|v| v.0 >= size) {
        return Err(PdfError::Malformed);
    }
    let result = CrossRef {
        at,
        table,
        size,
        root,
        info,
        id,
    };
    let root_at = reference_offset(bytes, &result, root, cancel)?;
    let mut header = Cursor {
        bytes: bytes.get(root_at..at).ok_or(PdfError::Malformed)?,
        at: 0,
    };
    if header.number()? != root.0 || header.number()? != u32::from(root.1) {
        return Err(PdfError::Malformed);
    }
    header.expect(b"obj")?;
    Ok(result)
}

fn reference_offset(
    bytes: &[u8],
    xref: &CrossRef<'_>,
    wanted: Reference,
    cancel: &PdfCancellation,
) -> PdfResult<usize> {
    let mut found = None;
    let mut cursor = Cursor {
        bytes: bytes.get(xref.at..).ok_or(PdfError::Malformed)?,
        at: 0,
    };
    table_end(&mut cursor, cancel, |object, generation, offset, live| {
        if (object, generation) == wanted && live {
            found = Some(offset as usize);
        }
        Ok(())
    })?;
    found.filter(|v| *v < xref.at).ok_or(PdfError::Malformed)
}

fn info_object(object: u32, metadata: &str) -> String {
    use std::fmt::Write;
    let mut info = format!("{object} 0 obj\n<< /Producer (VisualWorkbench) /Subject <FEFF");
    // Same serialized ASCII JSON as the existing image/TIFF metadata path,
    // encoded as a PDF UTF-16BE hex string (no PDF syntax can be injected).
    for byte in metadata.bytes() {
        let _ = write!(info, "00{byte:02X}");
    }
    info.push_str("> >>\nendobj\n");
    info
}

/// Embed caller-validated serialized export JSON into a completed private copy.
/// The caller must bind revision/source/capture fields before this call. This
/// never makes a redaction verified, and must precede independent verification.
/// Supports bounded, complete classic xref tables only. No silent fallback or
/// mutation of exact-original passthrough is permitted by the application.
pub fn embed_pdf_copy_metadata(
    bytes: &mut Vec<u8>,
    metadata: &str,
    limits: PdfLimits,
    cancel: &PdfCancellation,
) -> PdfResult<()> {
    metadata_valid(metadata)?;
    limits.validate()?;
    let xref = cross_ref(bytes, limits.output_bytes, cancel)?;
    let new_size = xref.size.checked_add(1).ok_or(PdfError::Limit)?;
    let object = info_object(xref.size, metadata);
    let info_at = xref.at.checked_add(1).ok_or(PdfError::Limit)?;
    let new_xref = info_at.checked_add(object.len()).ok_or(PdfError::Limit)?;
    let mut suffix = String::new();
    use std::fmt::Write;
    write!(
        suffix,
        "\n{} 1\n{info_at:010} 00000 n \ntrailer\n<< /Size {new_size} /Root {} {} R /Info {} 0 R",
        xref.size, xref.root.0, xref.root.1, xref.size
    )
    .map_err(|_| PdfError::Allocation)?;
    if let Some(id) = xref.id {
        suffix.push_str(" /ID ");
        suffix.push_str(std::str::from_utf8(id).map_err(|_| PdfError::Malformed)?);
    }
    write!(suffix, " >>\nstartxref\n{new_xref}\n%%EOF\n").map_err(|_| PdfError::Allocation)?;
    let length = new_xref
        .checked_add(xref.table.len())
        .and_then(|n| n.checked_add(suffix.len()))
        .ok_or(PdfError::Limit)?;
    if length as u64 > limits.output_bytes {
        return Err(PdfError::Limit);
    }
    let temporary = xref
        .table
        .len()
        .checked_add(object.len())
        .and_then(|n| n.checked_add(suffix.len()))
        .ok_or(PdfError::Limit)?;
    limits.admit_memory(bytes.capacity() as u64, temporary as u64)?;
    let mut table = Vec::new();
    table
        .try_reserve_exact(xref.table.len())
        .map_err(|_| PdfError::Allocation)?;
    table.extend_from_slice(xref.table);
    let truncate_at = xref.at;
    bytes
        .try_reserve(length.saturating_sub(bytes.len()))
        .map_err(|_| PdfError::Allocation)?;
    cancel.check()?;
    bytes.truncate(truncate_at);
    bytes.push(b'\n');
    bytes.extend_from_slice(object.as_bytes());
    bytes.extend_from_slice(&table);
    bytes.extend_from_slice(suffix.as_bytes());
    verify_pdf_copy_metadata(bytes, metadata, limits.output_bytes, cancel)
}

/// Read the active final trailer/xref and its authoritative /Info object.
/// A matching string hidden in source content, an orphan or an earlier revision
/// is not proof. Does not execute/load any native parser in the parent process.
pub fn verify_pdf_copy_metadata(
    bytes: &[u8],
    expected: &str,
    output_limit: u64,
    cancel: &PdfCancellation,
) -> PdfResult<()> {
    metadata_valid(expected)?;
    let xref = cross_ref(bytes, output_limit, cancel)?;
    let wanted = xref.info.ok_or(PdfError::Malformed)?;
    if wanted.1 != 0 {
        return Err(PdfError::Malformed);
    }
    let at = reference_offset(bytes, &xref, wanted, cancel)?;
    if bytes.get(at..xref.at) != Some(info_object(wanted.0, expected).as_bytes()) {
        return Err(PdfError::Malformed);
    }
    cancel.check()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    pub(crate) fn fixture() -> Vec<u8> {
        let objects: [&[u8]; 4] = [
            b"<< /Type /Catalog /Pages 2 0 R >>",
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>",
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 10 10] /Contents 4 0 R >>",
            b"<< /Length 0 >>\nstream\nendstream",
        ];
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            writeln!(bytes, "{} 0 obj", index + 1).unwrap();
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        writeln!(bytes, "xref\n0 5\n0000000000 65535 f ").unwrap();
        for at in offsets {
            writeln!(bytes, "{at:010} 00000 n ").unwrap();
        }
        write!(
            bytes,
            "trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        )
        .unwrap();
        bytes
    }
    pub(crate) fn identity() -> String {
        format!(
            r#"{{"schema_version":1,"host_seq":18446744073709551615,"state_hash":"aabbccdd{}","revision":"r18446744073709551615-aabbccdd","source_asset":"{}","capture_session":"00000000-0000-0000-0000-000000000001","frame_id":18446744073709551615}}"#,
            "a".repeat(56),
            "b".repeat(64)
        )
    }
    #[test]
    fn final_info_subject_contains_exact_json_and_page_bytes_are_unchanged() {
        let mut bytes = fixture();
        let prefix = bytes[..cross_ref(&bytes, u64::MAX, &PdfCancellation::default())
            .unwrap()
            .at]
            .to_vec();
        let metadata = identity();
        embed_pdf_copy_metadata(
            &mut bytes,
            &metadata,
            PdfLimits::default(),
            &PdfCancellation::default(),
        )
        .unwrap();
        assert!(bytes.starts_with(&prefix));
        verify_pdf_copy_metadata(&bytes, &metadata, u64::MAX, &PdfCancellation::default()).unwrap();
        // Independent byte-level hex decoding; native worker test also reads
        // Subject through PDFium after reopening both ordinary export modes.
        let text = std::str::from_utf8(&bytes).unwrap();
        let hex = text
            .split("/Subject <FEFF")
            .nth(1)
            .unwrap()
            .split('>')
            .next()
            .unwrap();
        let decoded: String = hex
            .as_bytes()
            .chunks_exact(4)
            .map(|unit| {
                let n = u16::from_str_radix(std::str::from_utf8(unit).unwrap(), 16).unwrap();
                char::from_u32(u32::from(n)).unwrap()
            })
            .collect();
        assert_eq!(decoded, metadata);
        assert_eq!(text.matches("startxref").count(), 1);
        assert!(!text.contains("/Prev"));
    }
    #[test]
    fn active_info_must_match_not_an_orphan_or_previous_value() {
        let mut bytes = fixture();
        let cancel = PdfCancellation::default();
        let first = identity();
        embed_pdf_copy_metadata(&mut bytes, &first, PdfLimits::default(), &cancel).unwrap();
        let second = first.replace("r18446744073709551615-aabbccdd", "r2-deadbeef");
        embed_pdf_copy_metadata(&mut bytes, &second, PdfLimits::default(), &cancel).unwrap();
        verify_pdf_copy_metadata(&bytes, &second, u64::MAX, &cancel).unwrap();
        assert!(verify_pdf_copy_metadata(&bytes, &first, u64::MAX, &cancel).is_err());
        let xref = cross_ref(&bytes, u64::MAX, &cancel).unwrap();
        let at = reference_offset(&bytes, &xref, xref.info.unwrap(), &cancel).unwrap();
        bytes[at] = b'9';
        assert!(verify_pdf_copy_metadata(&bytes, &second, u64::MAX, &cancel).is_err());
    }
    #[test]
    fn hostile_or_incremental_trailers_refuse_before_mutation() {
        for extra in [
            "/Prev 0",
            "/Encrypt 1 0 R",
            "/XRefStm 10",
            "/Size 5",
            "/Root 1 0 R",
            "/Unknown (private)",
        ] {
            let original = String::from_utf8(fixture())
                .unwrap()
                .replace("/Root 1 0 R >>", &format!("/Root 1 0 R {extra} >>"))
                .into_bytes();
            let mut bytes = original.clone();
            assert!(
                embed_pdf_copy_metadata(
                    &mut bytes,
                    &identity(),
                    PdfLimits::default(),
                    &PdfCancellation::default()
                )
                .is_err()
            );
            assert_eq!(bytes, original);
        }
    }
    #[test]
    fn final_xref_and_live_reference_are_required_not_matching_tail_text() {
        let cancel = PdfCancellation::default();
        let original = fixture();
        assert!(verify_pdf_copy_metadata(&original, &identity(), u64::MAX, &cancel).is_err());
        for bad in [
            original[..original.len() - 8].to_vec(),
            String::from_utf8(original.clone())
                .unwrap()
                .replace("/Root 1 0 R", "/Root 99 0 R")
                .into_bytes(),
            String::from_utf8(original.clone())
                .unwrap()
                .replace("xref\n0 5", "xref\n0 4294967295")
                .into_bytes(),
            String::from_utf8(original.clone())
                .unwrap()
                .replace("xref\n0 5", "xref\n4294967295 5")
                .into_bytes(),
        ] {
            assert!(verify_pdf_copy_metadata(&bad, &identity(), u64::MAX, &cancel).is_err());
        }
    }
    #[test]
    fn metadata_admission_and_cancel_refuse_without_writes() {
        for metadata in [
            String::new(),
            "private\0path".into(),
            "non-ascii \u{e9}".into(),
            "x".repeat(MAX_EXPORT_METADATA_BYTES + 1),
        ] {
            let original = fixture();
            let mut bytes = original.clone();
            assert!(
                embed_pdf_copy_metadata(
                    &mut bytes,
                    &metadata,
                    PdfLimits::default(),
                    &PdfCancellation::default()
                )
                .is_err()
            );
            assert_eq!(bytes, original);
        }
        let original = fixture();
        let mut bytes = original.clone();
        let cancel = PdfCancellation::default();
        cancel.cancel();
        assert!(matches!(
            embed_pdf_copy_metadata(&mut bytes, &identity(), PdfLimits::default(), &cancel),
            Err(PdfError::Cancelled)
        ));
        assert_eq!(bytes, original);
    }
    #[test]
    fn encoded_metadata_counts_toward_output_limit_and_maximum_is_supported() {
        let metadata = "x".repeat(MAX_EXPORT_METADATA_BYTES);
        let mut full = fixture();
        let cancel = PdfCancellation::default();
        embed_pdf_copy_metadata(&mut full, &metadata, PdfLimits::default(), &cancel).unwrap();
        verify_pdf_copy_metadata(&full, &metadata, u64::MAX, &cancel).unwrap();
        let limits = PdfLimits {
            output_bytes: full.len() as u64 - 1,
            ..PdfLimits::default()
        };
        let original = fixture();
        let mut bytes = original.clone();
        assert!(matches!(
            embed_pdf_copy_metadata(&mut bytes, &metadata, limits, &cancel),
            Err(PdfError::Limit)
        ));
        assert_eq!(bytes, original);
    }
}

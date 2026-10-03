use std::{
    borrow::Cow,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
use vw_raster::{
    DecodeLimits, DecodedImage, Pixels, PngSpool, RasterError, RasterSource, Region, SourceInfo,
    SpoolLimits, decode,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn unavailable_declared_icc_never_falls_back_to_untagged_or_srgb() -> TestResult {
    for srgb in [false, true] {
        let bytes = png_with_chunk(b"profile\0\x01not-a-supported-compression", srgb)?;
        refused_in_both_paths(&bytes)?;
    }
    Ok(())
}

#[test]
fn corrupt_and_truncated_icc_deflate_are_refused_before_scratch_changes() -> TestResult {
    for compressed in [
        b"".as_slice(),
        &[0],
        &[0x78, 0x9c],
        &[0x78, 0x9c, 0xff, 0xff, 0xff, 0xff],
    ] {
        let mut chunk = b"profile\0\0".to_vec();
        chunk.extend_from_slice(compressed);
        refused_in_both_paths(&png_with_chunk(&chunk, false)?)?;
    }
    Ok(())
}

#[test]
fn oversized_compressed_icc_is_rejected_before_any_clone_or_scratch_write() -> TestResult {
    let profile = vec![0u8; 5 * 1024 * 1024];
    let bytes = png_with_profile(&profile)?;
    assert!(
        bytes.len() < profile.len() / 8,
        "fixture must exercise compressed expansion"
    );
    refused_in_both_paths(&bytes)?;
    Ok(())
}

#[test]
fn localized_record_count_is_admitted_before_the_expanding_color_parser() -> TestResult {
    let profile = aliased_mluc(10_000, 65_536, 1)?;
    assert!(profile.len() < 200_000);
    assert!(matches!(
        decoded_with(profile.clone()).validate(),
        Err(RasterError::Unsupported("ICC localized text record limit"))
    ));
    assert!(matches!(
        source_info(profile.clone()).validate(),
        Err(RasterError::Unsupported("ICC localized text record limit"))
    ));
    refused_in_both_paths(&png_with_profile(&profile)?)?;
    Ok(())
}

#[test]
fn overlapping_text_payloads_and_repeated_tags_are_charged_repeatedly() -> TestResult {
    // Every record and all six known text fields reference the same 64 KiB.
    // Unique-byte accounting would admit it; owned parsed strings exceed cap.
    let profile = aliased_mluc(12, 65_536, 6)?;
    assert!(profile.len() < 70_000);
    assert!(matches!(
        decoded_with(profile.clone()).validate(),
        Err(RasterError::Unsupported("ICC expanded metadata limit"))
    ));
    assert!(matches!(
        source_info(profile.clone()).validate(),
        Err(RasterError::Unsupported("ICC expanded metadata limit"))
    ));
    refused_in_both_paths(&png_with_profile(&profile)?)?;
    Ok(())
}

#[test]
fn malformed_localized_offsets_fail_before_color_parser_allocation() -> TestResult {
    let mut profile = aliased_mluc(1, 16, 1)?;
    let tag = 144usize;
    profile[tag + 24..tag + 28].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(matches!(
        decoded_with(profile.clone()).validate(),
        Err(RasterError::Color)
    ));
    refused_in_both_paths(&png_with_profile(&profile)?)?;
    Ok(())
}

#[test]
fn admitted_profile_still_requires_its_parser_memory_in_the_caller_budget() -> TestResult {
    // Below the fixed ICC/tag limits, but its aliased text needs more than the
    // caller's 256 KiB. Reject before moxcms constructs those owned strings.
    let profile = aliased_mluc(8, 16_384, 1)?;
    let bytes = png_with_profile(&profile)?;
    let decode_limits = DecodeLimits {
        max_memory_bytes: 256 * 1024,
        ..DecodeLimits::default()
    };
    assert!(matches!(
        decode(&bytes, decode_limits),
        Err(RasterError::Memory { estimated, budget }) if estimated > budget
    ));
    let mut scratch = Scratch::new()?;
    assert!(matches!(
        PngSpool::decode(
            &bytes,
            scratch.file()?,
            SpoolLimits {
                decode: decode_limits,
                ..SpoolLimits::default()
            },
            &|| false,
        ),
        Err(RasterError::Memory { estimated, budget }) if estimated > budget
    ));
    assert_eq!(scratch.file()?.metadata()?.len(), 0);
    assert_eq!(scratch.file()?.stream_position()?, 0);
    Ok(())
}

#[test]
fn valid_icc_is_retained_byte_for_byte_in_buffered_and_spool_paths() -> TestResult {
    let profile = moxcms::ColorProfile::new_srgb().encode()?;
    let bytes = png_with_profile(&profile)?;
    let buffered = decode(&bytes, DecodeLimits::default())?;
    assert_eq!(buffered.icc.as_deref(), Some(profile.as_slice()));
    let mut scratch = Scratch::new()?;
    let mut spool = PngSpool::decode(&bytes, scratch.file()?, limits(), &|| false)?;
    assert_eq!(spool.info().icc.as_deref(), Some(profile.as_slice()));
    let region = spool.read_region(
        Region {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        64 * 1024 * 1024,
        &|| false,
    )?;
    assert_eq!(region.pixels, buffered.pixels);
    assert_eq!(region.icc, buffered.icc);
    Ok(())
}

#[test]
fn nonempty_caller_scratch_is_preserved_when_profile_admission_fails() -> TestResult {
    let bytes = png_with_chunk(b"profile\0\x01invalid", false)?;
    let mut scratch = Scratch::new()?;
    scratch.file()?.write_all(b"caller-owned sentinel")?;
    scratch.file()?.seek(SeekFrom::Start(3))?;
    assert!(PngSpool::decode(&bytes, scratch.file()?, limits(), &|| false).is_err());
    assert_eq!(scratch.file()?.stream_position()?, 3);
    scratch.file()?.seek(SeekFrom::Start(0))?;
    let mut retained = Vec::new();
    scratch.file()?.read_to_end(&mut retained)?;
    assert_eq!(retained, b"caller-owned sentinel");
    Ok(())
}

fn limits() -> SpoolLimits {
    SpoolLimits {
        decode: DecodeLimits {
            max_memory_bytes: 64 * 1024 * 1024,
            ..DecodeLimits::default()
        },
        ..SpoolLimits::default()
    }
}
fn refused_in_both_paths(bytes: &[u8]) -> TestResult {
    let original = bytes.to_vec();
    assert!(decode(bytes, limits().decode).is_err());
    let mut scratch = Scratch::new()?;
    assert!(PngSpool::decode(bytes, scratch.file()?, limits(), &|| false).is_err());
    assert_eq!(scratch.file()?.metadata()?.len(), 0);
    assert_eq!(scratch.file()?.stream_position()?, 0);
    assert_eq!(bytes, original);
    Ok(())
}
fn decoded_with(icc: Vec<u8>) -> DecodedImage {
    DecodedImage {
        width: 1,
        height: 1,
        pixels: Pixels::Rgba8(vec![10, 20, 30, 255]),
        icc: Some(icc),
        source_asset: vw_model::AssetId::hash(b"color admission fixture"),
        original_available: true,
        orientation_applied: 1,
    }
}
fn source_info(icc: Vec<u8>) -> SourceInfo {
    SourceInfo {
        width: 1,
        height: 1,
        bit_depth: 8,
        icc: Some(icc),
        source_asset: vw_model::AssetId::hash(b"color admission fixture"),
        original_available: true,
        orientation_applied: 1,
    }
}
fn png_with_chunk(chunk: &[u8], srgb: bool) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_chunk(png::chunk::iCCP, chunk)?;
    if srgb {
        writer.write_chunk(png::chunk::sRGB, &[0])?;
    }
    writer.write_image_data(&[10, 20, 30, 255])?;
    writer.finish()?;
    Ok(bytes)
}
fn png_with_profile(profile: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let mut info = png::Info::with_size(1, 1);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(Cow::Borrowed(profile));
    let encoder = png::Encoder::with_info(&mut bytes, info)?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&[10, 20, 30, 255])?;
    writer.finish()?;
    Ok(bytes)
}
fn aliased_mluc(
    records: u32,
    payload_bytes: u32,
    copies: u32,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let base = moxcms::ColorProfile::new_srgb().encode()?;
    let tag_offset = 132 + copies as usize * 12;
    let strings_offset = 16 + records as usize * 12;
    let tag_size = strings_offset + payload_bytes as usize;
    let mut profile = vec![0u8; tag_offset + tag_size];
    profile[..128].copy_from_slice(&base[..128]);
    let length = u32::try_from(profile.len())?;
    profile[..4].copy_from_slice(&length.to_be_bytes());
    profile[128..132].copy_from_slice(&copies.to_be_bytes());
    for (index, name) in [*b"desc", *b"cprt", *b"dmnd", *b"dmdd", *b"vued", *b"targ"]
        .into_iter()
        .take(copies as usize)
        .enumerate()
    {
        let start = 132 + index * 12;
        profile[start..start + 4].copy_from_slice(&name);
        profile[start + 4..start + 8].copy_from_slice(&(tag_offset as u32).to_be_bytes());
        profile[start + 8..start + 12].copy_from_slice(&(tag_size as u32).to_be_bytes());
    }
    let tag = &mut profile[tag_offset..];
    tag[..4].copy_from_slice(b"mluc");
    tag[8..12].copy_from_slice(&records.to_be_bytes());
    tag[12..16].copy_from_slice(&12u32.to_be_bytes());
    for index in 0..records as usize {
        let start = 16 + index * 12;
        tag[start..start + 4].copy_from_slice(b"enUS");
        tag[start + 4..start + 8].copy_from_slice(&payload_bytes.to_be_bytes());
        tag[start + 8..start + 12].copy_from_slice(&(strings_offset as u32).to_be_bytes());
    }
    for pair in tag[strings_offset..].as_chunks_mut::<2>().0.iter_mut() {
        pair.copy_from_slice(&[0, b'A']);
    }
    Ok(profile)
}

struct Scratch {
    file: Option<File>,
    path: PathBuf,
}
impl Scratch {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = if cfg!(target_os = "android") {
            std::env::current_dir()?
        } else {
            std::env::temp_dir()
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        let path = root.join(format!(
            "vw-color-admission-{}-{stamp}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self {
            file: Some(file),
            path,
        })
    }
    fn file(&mut self) -> Result<&mut File, Box<dyn std::error::Error>> {
        self.file.as_mut().ok_or_else(|| "closed scratch".into())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        drop(self.file.take());
        let _ = std::fs::remove_file(&self.path);
    }
}

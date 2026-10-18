//! Real worker/store/display regressions for the background-only native path.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::{borrow::Cow, io::Write, path::Path, sync::Arc};
use support::{TIME, cancel, device, id};
use vw_core::*;
use vw_model::AssetId;
use vw_raster::{AlphaPolicy, ColorPolicy, DecodedImage, ExportFormat, ExportRequest, Pixels};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
const BUDGET: u64 = 256 * 1024 * 1024;
const WIDTH: u32 = 3777;
const HEIGHT: u32 = 917;
static TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Copy)]
enum ColorTag {
    Srgb,
    Icc,
}

fn temp() -> std::result::Result<tempfile::TempDir, std::io::Error> {
    if cfg!(target_os = "android") {
        tempfile::tempdir_in(std::env::current_dir()?)
    } else {
        tempfile::tempdir()
    }
}

fn pattern(x: u32) -> [u8; 4] {
    match x % 4 {
        0 => [0, 0, 0, 0],
        1 => [255, 0, 0, 255],
        2 => [0, 128, 255, 128],
        _ => [255, 255, 255, 255],
    }
}

// The PNG encoder is the pinned png crate, not the rejected export/background
// round trip. A streamed repeated row avoids retaining a second full fixture.
fn png_original(
    depth: u8,
    color: ColorTag,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let profile = if matches!(color, ColorTag::Icc) {
        Some(vw_raster::standard_srgb_profile()?)
    } else {
        None
    };
    let mut info = png::Info::with_size(WIDTH, HEIGHT);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = if depth == 16 {
        png::BitDepth::Sixteen
    } else {
        png::BitDepth::Eight
    };
    if let Some(profile) = &profile {
        info.icc_profile = Some(Cow::Borrowed(profile));
    }
    let row: Vec<u8> = (0..WIDTH)
        .flat_map(|x| {
            pattern(x).into_iter().flat_map(move |value| {
                if depth == 16 {
                    (u16::from(value) * 257).to_be_bytes().to_vec()
                } else {
                    vec![value]
                }
            })
        })
        .collect();
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::with_info(&mut encoded, info)?;
        if matches!(color, ColorTag::Srgb) {
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        }
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Paeth);
        let mut writer = encoder.write_header()?;
        {
            let mut stream = writer.stream_writer()?;
            for _ in 0..HEIGHT {
                stream.write_all(&row)?;
            }
            stream.finish()?;
        }
        writer.finish()?;
    }
    Ok(encoded)
}

// Match the owner's original format/extent/depth/color declaration. The fixture
// encoder has its own 512MiB plan; background always receives the fixed256MiB.
fn jpeg_original() -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let image = DecodedImage {
        width: WIDTH,
        height: HEIGHT,
        pixels: Pixels::Rgba8([17, 83, 149, 255].repeat(WIDTH as usize * HEIGHT as usize)),
        icc: None,
        source_asset: AssetId::hash(b"synthetic untagged JPEG source pixels"),
        original_available: true,
        orientation_applied: 1,
    };
    Ok(vw_raster::export(
        &image,
        &ExportRequest {
            format: ExportFormat::Jpeg { quality: 95 },
            region: None,
            revision: vw_proto::v1::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: AlphaPolicy::Preserve,
            color: ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: 512 * 1024 * 1024,
            capture_session: None,
            frame_id: None,
        },
    )?
    .bytes)
}

async fn project(
    path: &Path,
    original: &[u8],
) -> std::result::Result<Arc<ProjectSession>, CoreError> {
    create_image_project(
        CreateImageProject {
            path: path.to_string_lossy().into(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: device(),
            title: "Synthetic background regression".into(),
            source: original.to_vec(),
            now_ms: TIME,
        },
        cancel(),
    )
    .await
}

async fn assert_unchanged(
    project: &Arc<ProjectSession>,
    path: &Path,
    original: &[u8],
    before: &ProjectInfo,
) -> TestResult {
    assert_eq!(&project.info().await?, before);
    let blobs = vw_store::BlobStore::new(path)?;
    let asset = AssetId::hash(original);
    assert_eq!(blobs.verify(&asset)?, original.len() as u64);
    assert_eq!(blobs.read(&asset)?, original);
    Ok(())
}

fn assert_pattern(image: &BackgroundImage, original: &[u8], depth: u8) -> TestResult {
    assert_eq!((image.width, image.height), (WIDTH, HEIGHT));
    assert_eq!(image.rgba.len(), WIDTH as usize * HEIGHT as usize * 4);
    assert_eq!(image.source_bit_depth, depth);
    assert_eq!(image.source_asset_id, AssetId::hash(original).to_string());
    assert_eq!(image.icc_profile, vw_raster::standard_srgb_profile()?);
    for index in [
        0,
        1,
        2,
        3,
        WIDTH as usize - 1,
        WIDTH as usize,
        WIDTH as usize + 2,
        WIDTH as usize * HEIGHT as usize / 2,
        WIDTH as usize * HEIGHT as usize - 1,
    ] {
        let expected = pattern((index % WIDTH as usize) as u32);
        let actual = &image.rgba[index * 4..index * 4 + 4];
        for channel in 0..3 {
            assert!(actual[channel].abs_diff(expected[channel]) <= 1);
        }
        assert_eq!(actual[3], expected[3]);
    }
    Ok(())
}

#[tokio::test]
async fn full_resolution_srgb_background_fits_256_mib_without_encoding() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(8, ColorTag::Srgb)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    let image = project
        .background_image(id(2), BUDGET, false, cancel())
        .await?;
    assert_pattern(&image, &original, 8)?;
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn tagged_icc_background_preserves_alpha_source_and_revision() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(8, ColorTag::Icc)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    let image = project
        .background_image(id(2), BUDGET, false, cancel())
        .await?;
    assert_pattern(&image, &original, 8)?;
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn untagged_jpeg_background_requires_explicit_consent_and_never_changes_original()
-> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = jpeg_original()?;
    let decoded = vw_raster::decode(&original, vw_raster::DecodeLimits::default())?;
    assert!(decoded.icc.is_none());
    assert_eq!(decoded.orientation_applied, 1);
    let expected = decoded.pixels.rgba8();
    drop(decoded);
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancel())
            .await,
        Err(CoreError::Raster)
    ));
    assert_unchanged(&project, &path, &original, &before).await?;
    let image = project
        .background_image(id(2), BUDGET, true, cancel())
        .await?;
    assert_eq!((image.width, image.height), (WIDTH, HEIGHT));
    assert_eq!(image.source_bit_depth, 8);
    assert_eq!(image.source_asset_id, AssetId::hash(&original).to_string());
    assert_eq!(image.icc_profile, vw_raster::standard_srgb_profile()?);
    assert_eq!(image.rgba.len(), expected.len());
    assert!(
        image
            .rgba
            .iter()
            .zip(&expected)
            .all(|(a, b)| a.abs_diff(*b) <= 1)
    );
    assert!(
        image
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|rgba| rgba[3] == 255)
    );
    assert_unchanged(&project, &path, &original, &before).await?;
    // An earlier consented derivative cannot silently tag or change the source.
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancel())
            .await,
        Err(CoreError::Raster)
    ));
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn sixteen_bit_background_reduces_display_only_and_preserves_source() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(16, ColorTag::Srgb)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    let image = project
        .background_image(id(2), BUDGET, false, cancel())
        .await?;
    assert_pattern(&image, &original, 16)?;
    assert_eq!(
        project.document_snapshot(id(2), cancel()).await?.bit_depth,
        16
    );
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn insufficient_background_budget_is_backpressure_and_retry_succeeds() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(8, ColorTag::Srgb)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    assert!(matches!(
        project
            .background_image(id(2), 160 * 1024 * 1024, false, cancel())
            .await,
        Err(CoreError::Backpressure)
    ));
    for budget in [0, BUDGET + 1] {
        assert!(matches!(
            project
                .background_image(id(2), budget, false, cancel())
                .await,
            Err(CoreError::Invalid)
        ));
    }
    assert_unchanged(&project, &path, &original, &before).await?;
    let image = project
        .background_image(id(2), BUDGET, false, cancel())
        .await?;
    assert_pattern(&image, &original, 8)?;
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn corrupt_or_missing_original_is_refused_without_changing_revision() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(8, ColorTag::Srgb)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    let blobs = vw_store::BlobStore::new(&path)?;
    let blob = blobs.path(&AssetId::hash(&original))?;
    let mut damaged = original.clone();
    damaged[40] ^= 1;
    std::fs::write(&blob, &damaged)?;
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancel())
            .await,
        Err(CoreError::Raster)
    ));
    assert_eq!(project.info().await?, before);
    std::fs::write(&blob, &original)?;
    assert_unchanged(&project, &path, &original, &before).await?;
    let held = directory.path().join("held-original.png");
    std::fs::rename(&blob, &held)?;
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancel())
            .await,
        Err(CoreError::Raster)
    ));
    assert_eq!(project.info().await?, before);
    std::fs::rename(&held, &blob)?;
    let image = project
        .background_image(id(2), BUDGET, false, cancel())
        .await?;
    assert_pattern(&image, &original, 8)?;
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    Ok(())
}

#[tokio::test]
async fn background_cancellation_and_closed_session_do_not_publish_pixels() -> TestResult {
    let _serial = TESTS.lock().await;
    let directory = temp()?;
    let path = directory.path().join("project");
    let original = png_original(8, ColorTag::Srgb)?;
    let project = project(&path, &original).await?;
    let before = project.info().await?;
    let cancelled = cancel();
    cancelled.cancel();
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancelled)
            .await,
        Err(CoreError::Cancelled)
    ));
    assert_unchanged(&project, &path, &original, &before).await?;
    project.close().await?;
    assert!(matches!(
        project
            .background_image(id(2), BUDGET, false, cancel())
            .await,
        Err(CoreError::Closed)
    ));
    assert_eq!(
        vw_store::BlobStore::new(&path)?.read(&AssetId::hash(&original))?,
        original
    );
    Ok(())
}

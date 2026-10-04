#![allow(dead_code)]
use vw_ai::{config::*, *};
use vw_mask::{Mask, Size};

pub type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
pub fn provider() -> ProviderConfig {
    ProviderConfig {
        schema: 1,
        kind: ProviderKind::OpenAiImageEdits,
        verified_on: "2026-10-02".into(),
        endpoint: "https://api.openai.com/v1/images/edits".into(),
        model: "offline-fixture-model".into(),
        quality: "offline-fixture-quality".into(),
        capabilities: Capabilities {
            mask_support: MaskSupport::AlphaPngGuidance,
            max_long_edge: 256,
            min_pixels: 16 * 16,
            max_pixels: 256 * 256,
            size_multiple: 16,
            max_aspect_ratio: 3,
            formats: vec![Format::Png],
            max_image_bytes_exclusive: MAX_ENCODED,
            max_mask_bytes_exclusive: MAX_ENCODED,
            experimental_above_pixels: 128 * 128,
        },
        prices: Prices {
            text_input_microusd_per_million: 1_000_000,
            image_input_microusd_per_million: 2_000_000,
            image_output_microusd_per_million: 3_000_000,
        },
        timeout_seconds: 60,
        max_response_bytes: 10 * 1024 * 1024,
    }
}
pub fn options(intent: u8) -> std::result::Result<PrepareOptions, vw_model::ModelError> {
    Ok(PrepareOptions {
        provider: provider(),
        revision: RevisionBinding {
            project_id: vw_model::Id::from_parts(1, [0; 10])?,
            host_seq: 7,
            state_hash: "a".repeat(64),
        },
        intent_id: vw_model::Id::from_parts(1, [intent; 10])?,
        instructions: vec![Instruction {
            role: InstructionRole::Change,
            text: "Use the fixture color inside the selection.".into(),
        }],
        feather_px: 2,
        estimated_tokens: Some(Tokens {
            text_input: 10,
            image_input: 20,
            image_output: 30,
        }),
        source_policy: SourcePolicy {
            assume_untagged_srgb: true,
            allow_16bit_provider_copy: false,
        },
        limits: Limits::default(),
    })
}
pub fn png8(
    width: u32,
    height: u32,
    mut pixel: impl FnMut(u32, u32) -> [u8; 4],
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&pixel(x, y));
        }
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&data)?;
        writer.finish()?;
    }
    Ok(bytes)
}
pub fn png16(width: u32, height: u32) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let data = (0..width * height)
        .flat_map(|i| {
            [
                1001 + (i % 101) as u16,
                2345,
                65001,
                40001 + (i % 31) as u16,
            ]
        })
        .flat_map(u16::to_be_bytes)
        .collect::<Vec<_>>();
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&data)?;
        writer.finish()?;
    }
    Ok(bytes)
}
pub fn region(
    width: u32,
    height: u32,
    left: u32,
    top: u32,
    right: u32,
    bottom: u32,
) -> std::result::Result<Mask, vw_mask::MaskError> {
    let mut data = vec![0; width as usize * height as usize];
    for y in top..bottom {
        for x in left..right {
            data[(y * width + x) as usize] = 255;
        }
    }
    Mask::from_dense(Size::new(width, height)?, &data)
}
pub fn prepared(intent: u8) -> std::result::Result<Prepared, Box<dyn std::error::Error>> {
    let source = png8(64, 64, |x, y| [(x * 3) as u8, (y * 3) as u8, 70, 180])?;
    Ok(Prepared::new(
        &source,
        &[region(64, 64, 24, 24, 40, 40)?],
        options(intent)?,
        &NeverCancel,
    )?)
}
pub fn temp() -> std::io::Result<tempfile::TempDir> {
    let base = if cfg!(target_os = "android") {
        std::env::current_dir()?
    } else {
        std::env::temp_dir()
    };
    tempfile::Builder::new()
        .prefix("vw-ai-owned-")
        .tempdir_in(base)
}
pub fn read_rgba(bytes: &[u8]) -> std::result::Result<image::RgbaImage, image::ImageError> {
    Ok(image::load_from_memory(bytes)?.to_rgba8())
}

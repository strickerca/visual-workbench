//! Full-every-page raster reconstruction for phone-only redaction. A normal
//! annotated/flattened PDF never enters this verification/receipt path.
use crate::{
    PageGeometry, PdfCancellation, PdfDocument, PdfError, PdfLimits, PdfNotice, PdfRect,
    PdfTileRequest,
};
use std::io::Write;

const MAX_REGIONS: usize = 4096;
const MAX_RASTER_PIXELS: u64 = 100_000_000;
const MAX_RASTER_OBJECTS: u32 = 100_000;
const MAX_REDACTION_WORK: u64 = 1_000_000_000;
const TILE: u32 = 512;

#[derive(Debug, thiserror::Error)]
pub enum RasterRedactionError {
    #[error(transparent)]
    Pdf(#[from] PdfError),
    #[error("invalid raster redaction request")]
    Invalid,
    #[error("raster redaction work or output limit exceeded")]
    Limit,
    #[error("redaction source or prepared page geometry changed")]
    SourceMismatch,
    #[error("raster redaction verification failed")]
    Verification,
}
pub type RasterRedactionResult<T> = Result<T, RasterRedactionError>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageRedaction {
    pub page: u32,
    pub region: PdfRect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterSecurityPolicy {
    RefuseEncrypted,
    ExplicitlyRemoveEncryption,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RasterRedactionOptions {
    /// Explicit requested resolution. The prepared plan reports the nearest f32
    /// scale actually used by PDFium. No downsampling to fit a budget occurs.
    pub pixels_per_point: f64,
    pub security: RasterSecurityPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterWork {
    pub pages: u32,
    pub pixels: u64,
    pub tiles: u32,
    pub pdf_objects: u32,
    pub maximum_encoded_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RasterProgress {
    pub pages_complete: u32,
    pub tiles_complete: u32,
    pub total_tiles: u32,
}

#[derive(Clone, Copy, Debug)]
struct RasterPage {
    geometry: PageGeometry,
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
    page_object: u32,
}
impl RasterPage {
    fn tiles(self) -> u32 {
        self.columns * self.rows
    }
    fn content_object(self) -> u32 {
        self.page_object + 1
    }
    fn first_image(self) -> u32 {
        self.page_object + 2
    }
}

/// Immutable source/resolution/rectangle binding. Prepares before the first
/// render or output byte, including full-document output and wrapper admission.
pub struct RasterRedactionPlan {
    source_blake3: String,
    plan_blake3: String,
    pages: Vec<RasterPage>,
    regions: Vec<PageRedaction>,
    options: RasterRedactionOptions,
    requested_pixels_per_point: f64,
    work: RasterWork,
    additional_memory: u64,
}

#[derive(Debug)]
pub struct UnverifiedRasterCopy {
    source_blake3: String,
    plan_blake3: String,
    output_blake3: String,
    bytes: u64,
}
impl UnverifiedRasterCopy {
    pub fn output_blake3(&self) -> &str {
        &self.output_blake3
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedRasterRedaction {
    source_blake3: String,
    output_blake3: String,
    plan_blake3: String,
    pages: u32,
    regions: u32,
    bytes: u64,
    pixels_per_point: f64,
    requested_pixels_per_point: f64,
}
impl VerifiedRasterRedaction {
    pub fn source_blake3(&self) -> &str {
        &self.source_blake3
    }
    pub fn output_blake3(&self) -> &str {
        &self.output_blake3
    }
    pub fn plan_blake3(&self) -> &str {
        &self.plan_blake3
    }
    pub fn pages(&self) -> u32 {
        self.pages
    }
    pub fn regions(&self) -> u32 {
        self.regions
    }
    pub fn bytes(&self) -> u64 {
        self.bytes
    }
    pub fn pixels_per_point(&self) -> f64 {
        self.pixels_per_point
    }
    pub fn requested_pixels_per_point(&self) -> f64 {
        self.requested_pixels_per_point
    }
}

/// Nonallocating whole-document admission, usable before decoding page pixels.
/// Generated output must fit the input limit too, so verification cannot fail
/// predictably only after it has written an oversized private PDF.
pub fn admit_raster_redaction(
    sizes: &[[f64; 2]],
    scale: f64,
    region_count: usize,
    limits: PdfLimits,
) -> RasterRedactionResult<RasterWork> {
    let limits = limits.validate()?;
    if sizes.is_empty()
        || sizes.len() as u64 > u64::from(limits.pages)
        || region_count == 0
        || region_count > MAX_REGIONS
        || !scale.is_finite()
        || !(0.25..=8.0).contains(&scale)
    {
        return Err(RasterRedactionError::Invalid);
    }
    let scale = effective_scale(scale)?;
    let mut pixels = 0_u64;
    let mut tiles = 0_u32;
    for [width, height] in sizes.iter().copied() {
        let (width, height) = raster_size(width, height, scale, limits)?;
        pixels = pixels
            .checked_add(u64::from(width) * u64::from(height))
            .ok_or(RasterRedactionError::Limit)?;
        let count = width
            .div_ceil(TILE)
            .checked_mul(height.div_ceil(TILE))
            .ok_or(RasterRedactionError::Limit)?;
        if count > limits.objects_per_page {
            return Err(RasterRedactionError::Limit);
        }
        tiles = tiles
            .checked_add(count)
            .ok_or(RasterRedactionError::Limit)?;
        if pixels > MAX_RASTER_PIXELS {
            return Err(RasterRedactionError::Limit);
        }
    }
    let pages = u32::try_from(sizes.len()).map_err(|_| RasterRedactionError::Limit)?;
    let objects = tiles
        .checked_add(pages.checked_mul(2).ok_or(RasterRedactionError::Limit)?)
        .and_then(|n| n.checked_add(2))
        .ok_or(RasterRedactionError::Limit)?;
    if objects > MAX_RASTER_OBJECTS {
        return Err(RasterRedactionError::Limit);
    }
    // Each RGB image is uncompressed, and no arbitrary source string is copied.
    // 512 bytes/tile +4KiB/page exceed all bounded numeric names/matrices/xref
    // overhead. No original catalog, metadata, resource or text is embedded.
    let encoded = pixels
        .checked_mul(3)
        .and_then(|n| n.checked_add(u64::from(tiles) * 512))
        .and_then(|n| n.checked_add(u64::from(pages) * 4096))
        .and_then(|n| n.checked_add(64 * 1024))
        .ok_or(RasterRedactionError::Limit)?;
    if encoded > limits.output_bytes.min(limits.input_bytes) {
        return Err(RasterRedactionError::Limit);
    }
    Ok(RasterWork {
        pages,
        pixels,
        tiles,
        pdf_objects: objects,
        maximum_encoded_bytes: encoded,
    })
}

fn raster_size(
    width: f64,
    height: f64,
    scale: f64,
    limits: PdfLimits,
) -> RasterRedactionResult<(u32, u32)> {
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || width > limits.page_side_points
        || height > limits.page_side_points
    {
        return Err(RasterRedactionError::Invalid);
    }
    // Checked page/scale limits keep both products far below u32::MAX. Use the
    // same outward multiplication as redaction coverage, including exact edges.
    Ok((pixel_upper(width, scale), pixel_upper(height, scale)))
}

fn effective_scale(requested: f64) -> RasterRedactionResult<f64> {
    if !requested.is_finite() || !(0.25..=8.0).contains(&requested) {
        return Err(RasterRedactionError::Invalid);
    }
    Ok(f64::from(requested as f32))
}

fn tile_origin(pixel: u32, scale: f64) -> RasterRedactionResult<f64> {
    let origin = f64::from(pixel) / scale;
    // Match ffi::render_tile's actual FS_MATRIX translation exactly. Native
    // scale and integer pixel origins now define one grid across all seams.
    if f64::from((-origin * scale) as f32) != -f64::from(pixel) {
        return Err(RasterRedactionError::Invalid);
    }
    Ok(origin)
}

impl RasterRedactionPlan {
    pub fn prepare(
        document: &mut PdfDocument<'_>,
        regions: Vec<PageRedaction>,
        options: RasterRedactionOptions,
        cancel: &PdfCancellation,
    ) -> RasterRedactionResult<Self> {
        cancel.check()?;
        let limits = document.redaction_limits();
        if regions.is_empty() || regions.len() > MAX_REGIONS {
            return Err(RasterRedactionError::Invalid);
        }
        if !options.pixels_per_point.is_finite()
            || !(0.25..=8.0).contains(&options.pixels_per_point)
        {
            return Err(RasterRedactionError::Invalid);
        }
        let requested_pixels_per_point = options.pixels_per_point;
        let options = RasterRedactionOptions {
            pixels_per_point: effective_scale(requested_pixels_per_point)?,
            ..options
        };
        if document.info().encrypted
            && options.security != RasterSecurityPolicy::ExplicitlyRemoveEncryption
        {
            return Err(PdfError::EncryptedExport.into());
        }
        if document
            .info()
            .notices
            .contains(&PdfNotice::NonInteractiveForms)
        {
            return Err(PdfError::UnsupportedForms.into());
        }
        let page_count = document.info().pages;
        let base_memory = (regions.capacity() as u64)
            .checked_mul(std::mem::size_of::<PageRedaction>() as u64)
            .and_then(|n| {
                n.checked_add(
                    u64::from(page_count)
                        * (std::mem::size_of::<RasterPage>() + std::mem::size_of::<[f64; 2]>())
                            as u64,
                )
            })
            .ok_or(RasterRedactionError::Limit)?;
        document.admit_redaction_memory(base_memory)?;
        let mut pages = Vec::new();
        let mut sizes = Vec::new();
        pages
            .try_reserve_exact(page_count as usize)
            .map_err(|_| PdfError::Allocation)?;
        sizes
            .try_reserve_exact(page_count as usize)
            .map_err(|_| PdfError::Allocation)?;
        let mut next_object = 3_u32;
        for page in 0..page_count {
            cancel.check()?;
            let geometry = document.page_info(page, cancel)?.geometry;
            let (width, height) = raster_size(
                geometry.width(),
                geometry.height(),
                options.pixels_per_point,
                limits,
            )?;
            let (columns, rows) = (width.div_ceil(TILE), height.div_ceil(TILE));
            // Admission includes every actual native translation before output.
            for column in 0..columns {
                tile_origin(column * TILE, options.pixels_per_point)?;
            }
            for row in 0..rows {
                tile_origin(row * TILE, options.pixels_per_point)?;
            }
            let count = columns
                .checked_mul(rows)
                .ok_or(RasterRedactionError::Limit)?;
            if count > limits.objects_per_page {
                return Err(RasterRedactionError::Limit);
            }
            pages.push(RasterPage {
                geometry,
                width,
                height,
                columns,
                rows,
                page_object: next_object,
            });
            sizes.push([geometry.width(), geometry.height()]);
            next_object = next_object
                .checked_add(count)
                .and_then(|n| n.checked_add(2))
                .ok_or(RasterRedactionError::Limit)?;
            if next_object > MAX_RASTER_OBJECTS + 1 {
                return Err(RasterRedactionError::Limit);
            }
        }
        let work = admit_raster_redaction(&sizes, options.pixels_per_point, regions.len(), limits)?;
        admit_regions(&pages, &regions, options.pixels_per_point, work, cancel)?;
        let max_page_tiles = pages
            .iter()
            .map(|page| page.tiles())
            .max()
            .ok_or(RasterRedactionError::Invalid)?;
        let additional_memory = base_memory
            .checked_add(u64::from(TILE) * u64::from(TILE) * 7)
            .and_then(|n| n.checked_add(u64::from(work.pdf_objects + 1) * 8))
            .and_then(|n| n.checked_add(u64::from(max_page_tiles) * 128))
            .ok_or(RasterRedactionError::Limit)?;
        document.admit_redaction_memory(additional_memory)?;
        // The output will be reopened while this plan is still retained.
        // Refuse a predictable verification admission failure before rendering.
        limits.admit_memory(work.maximum_encoded_bytes, additional_memory)?;
        let source_blake3 = document.info().source_blake3.clone();
        let mut binding = blake3::Hasher::new();
        binding.update(b"vw-full-raster-redaction-v2\0");
        binding.update(source_blake3.as_bytes());
        binding.update(&requested_pixels_per_point.to_bits().to_le_bytes());
        binding.update(&options.pixels_per_point.to_bits().to_le_bytes());
        binding.update(&[match options.security {
            RasterSecurityPolicy::RefuseEncrypted => 0,
            RasterSecurityPolicy::ExplicitlyRemoveEncryption => 1,
        }]);
        for page in &pages {
            binding.update(&page.geometry.width().to_bits().to_le_bytes());
            binding.update(&page.geometry.height().to_bits().to_le_bytes());
        }
        for region in &regions {
            binding.update(&region.page.to_le_bytes());
            for value in [
                region.region.left,
                region.region.top,
                region.region.right,
                region.region.bottom,
            ] {
                binding.update(&value.to_bits().to_le_bytes());
            }
        }
        let plan_blake3 = binding.finalize().to_hex().to_string();
        cancel.check()?;
        Ok(Self {
            source_blake3,
            plan_blake3,
            pages,
            regions,
            options,
            requested_pixels_per_point,
            work,
            additional_memory,
        })
    }

    pub fn work(&self) -> RasterWork {
        self.work
    }
    pub fn plan_blake3(&self) -> &str {
        &self.plan_blake3
    }
    pub fn requested_pixels_per_point(&self) -> f64 {
        self.requested_pixels_per_point
    }
    /// Exact f32 render-matrix value, exposed as f64 without further rounding.
    pub fn pixels_per_point(&self) -> f64 {
        self.options.pixels_per_point
    }

    /// Every page is reconstructed. Only sanitized RGB samples and generated
    /// numeric PDF syntax enter the private writer. This is still unverified.
    pub fn write_unverified(
        &self,
        document: &mut PdfDocument<'_>,
        writer: &mut dyn Write,
        cancel: &PdfCancellation,
        progress: &mut dyn FnMut(RasterProgress),
    ) -> RasterRedactionResult<UnverifiedRasterCopy> {
        cancel.check()?;
        self.check_source(document, cancel)?;
        document.admit_redaction_memory(self.additional_memory)?;
        let mut output = FreshWriter::new(
            writer,
            self.work.maximum_encoded_bytes,
            self.work.pdf_objects,
            cancel,
        )?;
        output.bytes(b"%PDF-1.7\n%VW raster reconstruction\n")?;
        output.begin(1)?;
        output.bytes(b"<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")?;
        output.begin(2)?;
        output.text(format_args!(
            "<< /Type /Pages /Count {} /Kids [",
            self.work.pages
        ))?;
        for page in &self.pages {
            output.text(format_args!("{} 0 R ", page.page_object))?;
        }
        output.bytes(b"] >>\nendobj\n")?;
        let mut tiles_complete = 0_u32;
        for (page_index, page) in self.pages.iter().copied().enumerate() {
            cancel.check()?;
            let content = content_stream(page, self.options.pixels_per_point)?;
            output.begin(page.page_object)?;
            output.text(format_args!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.8} {:.8}] /Resources << /XObject << ", page.geometry.width(), page.geometry.height()))?;
            for tile in 0..page.tiles() {
                output.text(format_args!("/T{tile} {} 0 R ", page.first_image() + tile))?;
            }
            output.text(format_args!(
                ">> >> /Contents {} 0 R >>\nendobj\n",
                page.content_object()
            ))?;
            output.begin(page.content_object())?;
            output.text(format_args!("<< /Length {} >>\nstream\n", content.len()))?;
            output.bytes(&content)?;
            output.bytes(b"endstream\nendobj\n")?;
            drop(content);
            for row in 0..page.rows {
                for column in 0..page.columns {
                    cancel.check()?;
                    let (x, y) = (column * TILE, row * TILE);
                    let (width, height) = (TILE.min(page.width - x), TILE.min(page.height - y));
                    let mut tile = document.render_tile(
                        page_index as u32,
                        PdfTileRequest {
                            x: tile_origin(x, self.options.pixels_per_point)?,
                            y: tile_origin(y, self.options.pixels_per_point)?,
                            pixels_per_point: self.options.pixels_per_point,
                            width,
                            height,
                            include_annotations: true,
                        },
                        cancel,
                    )?;
                    replace_pixels(
                        &mut tile.rgba,
                        width,
                        height,
                        x,
                        y,
                        page_index as u32,
                        self.options.pixels_per_point,
                        &self.regions,
                        cancel,
                    )?;
                    let rgb_len = (width as usize)
                        .checked_mul(height as usize)
                        .and_then(|n| n.checked_mul(3))
                        .ok_or(RasterRedactionError::Limit)?;
                    let mut rgb = Vec::new();
                    rgb.try_reserve_exact(rgb_len)
                        .map_err(|_| PdfError::Allocation)?;
                    for pixel in tile.rgba.as_chunks::<4>().0.iter() {
                        if pixel[3] != 255 {
                            return Err(RasterRedactionError::Verification);
                        }
                        rgb.extend_from_slice(&pixel[..3]);
                    }
                    output.begin(page.first_image() + row * page.columns + column)?;
                    output.text(format_args!("<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {rgb_len} >>\nstream\n"))?;
                    output.bytes(&rgb)?;
                    output.bytes(b"\nendstream\nendobj\n")?;
                    tiles_complete += 1;
                    progress(RasterProgress {
                        pages_complete: page_index as u32,
                        tiles_complete,
                        total_tiles: self.work.tiles,
                    });
                }
            }
            progress(RasterProgress {
                pages_complete: page_index as u32 + 1,
                tiles_complete,
                total_tiles: self.work.tiles,
            });
        }
        let (bytes, output_blake3) = output.finish()?;
        cancel.check()?;
        Ok(UnverifiedRasterCopy {
            source_blake3: self.source_blake3.clone(),
            plan_blake3: self.plan_blake3.clone(),
            output_blake3,
            bytes,
        })
    }

    /// Reopen the completed private file, after closing the original document.
    /// This consumes its intermediate receipt. Changed bytes cannot pass, even
    /// if a different PDF happens to contain only image objects and no text.
    pub fn verify(
        &self,
        document: &mut PdfDocument<'_>,
        intermediate: UnverifiedRasterCopy,
        cancel: &PdfCancellation,
    ) -> RasterRedactionResult<VerifiedRasterRedaction> {
        cancel.check()?;
        if intermediate.source_blake3 != self.source_blake3
            || intermediate.plan_blake3 != self.plan_blake3
            || document.info().source_blake3 != intermediate.output_blake3
            || document.info().pages != self.work.pages
            || document.info().encrypted
            || document
                .info()
                .notices
                .contains(&PdfNotice::NonInteractiveForms)
        {
            return Err(RasterRedactionError::Verification);
        }
        document.admit_redaction_memory(self.additional_memory)?;
        for (index, expected) in self.pages.iter().enumerate() {
            cancel.check()?;
            let info = document.page_info(index as u32, cancel)?;
            let geometry = info.geometry;
            if geometry.quarter_turns() != 0
                || geometry.crop_pdf()[0] != 0.0
                || geometry.crop_pdf()[1] != 0.0
                || (geometry.width() - expected.geometry.width()).abs() > 0.01
                || (geometry.height() - expected.geometry.height()).abs() > 0.01
            {
                return Err(RasterRedactionError::Verification);
            }
            document.verify_raster_page(index as u32, expected.tiles(), cancel)?;
        }
        cancel.check()?;
        Ok(VerifiedRasterRedaction {
            source_blake3: self.source_blake3.clone(),
            output_blake3: intermediate.output_blake3,
            plan_blake3: self.plan_blake3.clone(),
            pages: self.work.pages,
            regions: self.regions.len() as u32,
            bytes: intermediate.bytes,
            pixels_per_point: self.options.pixels_per_point,
            requested_pixels_per_point: self.requested_pixels_per_point,
        })
    }

    fn check_source(
        &self,
        document: &mut PdfDocument<'_>,
        cancel: &PdfCancellation,
    ) -> RasterRedactionResult<()> {
        if document.info().source_blake3 != self.source_blake3
            || document.info().pages != self.work.pages
        {
            return Err(RasterRedactionError::SourceMismatch);
        }
        for (index, expected) in self.pages.iter().enumerate() {
            if document.page_info(index as u32, cancel)?.geometry != expected.geometry {
                return Err(RasterRedactionError::SourceMismatch);
            }
        }
        Ok(())
    }
}

fn admit_regions(
    pages: &[RasterPage],
    regions: &[PageRedaction],
    scale: f64,
    work: RasterWork,
    cancel: &PdfCancellation,
) -> RasterRedactionResult<()> {
    let mut replacement_work = work
        .pixels
        .checked_add(
            u64::from(work.tiles)
                .checked_mul(regions.len() as u64)
                .ok_or(RasterRedactionError::Limit)?,
        )
        .ok_or(RasterRedactionError::Limit)?;
    for region in regions {
        cancel.check()?;
        region.region.validate()?;
        let page = pages
            .get(region.page as usize)
            .ok_or(RasterRedactionError::Invalid)?;
        if region.region.left < 0.0
            || region.region.top < 0.0
            || region.region.right > page.geometry.width()
            || region.region.bottom > page.geometry.height()
            || region.region.left >= region.region.right
            || region.region.top >= region.region.bottom
        {
            return Err(RasterRedactionError::Invalid);
        }
        let (left, top) = (
            pixel_lower(region.region.left, scale),
            pixel_lower(region.region.top, scale),
        );
        let (right, bottom) = (
            pixel_upper(region.region.right, scale).min(page.width),
            pixel_upper(region.region.bottom, scale).min(page.height),
        );
        replacement_work = replacement_work
            .checked_add(
                u64::from(right.saturating_sub(left)) * u64::from(bottom.saturating_sub(top)),
            )
            .ok_or(RasterRedactionError::Limit)?;
        if replacement_work > MAX_REDACTION_WORK {
            return Err(RasterRedactionError::Limit);
        }
    }
    Ok(())
}

fn content_stream(page: RasterPage, scale: f64) -> RasterRedactionResult<Vec<u8>> {
    let mut result = Vec::new();
    result
        .try_reserve_exact(page.tiles() as usize * 128)
        .map_err(|_| PdfError::Allocation)?;
    for row in 0..page.rows {
        for column in 0..page.columns {
            let (x, y) = (column * TILE, row * TILE);
            let (width, height) = (TILE.min(page.width - x), TILE.min(page.height - y));
            writeln!(
                &mut result,
                "q {:.8} 0 0 {:.8} {:.8} {:.8} cm /T{} Do Q",
                f64::from(width) / scale,
                f64::from(height) / scale,
                f64::from(x) / scale,
                page.geometry.height() - f64::from(y + height) / scale,
                row * page.columns + column
            )
            .map_err(|error| PdfError::Io(error.kind()))?;
        }
    }
    if result.len() > page.tiles() as usize * 128 {
        return Err(RasterRedactionError::Limit);
    }
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn replace_pixels(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    tile_x: u32,
    tile_y: u32,
    page: u32,
    scale: f64,
    regions: &[PageRedaction],
    cancel: &PdfCancellation,
) -> RasterRedactionResult<()> {
    if rgba.len() != width as usize * height as usize * 4 {
        return Err(RasterRedactionError::Verification);
    }
    for region in regions.iter().filter(|region| region.page == page) {
        cancel.check()?;
        // Every pixel whose area intersects a redaction rectangle is replaced,
        // including fractional edges and tile seams. No original hidden RGB is
        // retained behind alpha, in a soft mask, or under a black overlay object.
        let left = pixel_lower(region.region.left, scale);
        let top = pixel_lower(region.region.top, scale);
        let right = pixel_upper(region.region.right, scale);
        let bottom = pixel_upper(region.region.bottom, scale);
        let x0 = left.max(tile_x).min(tile_x + width) - tile_x;
        let y0 = top.max(tile_y).min(tile_y + height) - tile_y;
        let x1 = right.max(tile_x).min(tile_x + width) - tile_x;
        let y1 = bottom.max(tile_y).min(tile_y + height) - tile_y;
        for y in y0..y1 {
            cancel.check()?;
            for x in x0..x1 {
                let index = ((y * width + x) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&[0, 0, 0, 255]);
            }
        }
    }
    Ok(())
}

// FMA recovers the multiplication rounding direction so a sub-pixel region
// cannot collapse onto an exact integer boundary and leave its pixel intact.
// Exact boundaries stay exact; there is no unconditional one-pixel halo.
fn pixel_lower(value: f64, scale: f64) -> u32 {
    let product = value * scale;
    let residual = value.mul_add(scale, -product);
    let lower = if residual < 0.0 && product > 0.0 {
        f64::from_bits(product.to_bits() - 1)
    } else {
        product
    };
    lower.floor() as u32
}
fn pixel_upper(value: f64, scale: f64) -> u32 {
    let product = value * scale;
    if value > 0.0 && product == 0.0 {
        return 1;
    }
    let residual = value.mul_add(scale, -product);
    let upper = if residual > 0.0 {
        f64::from_bits(product.to_bits() + 1)
    } else {
        product
    };
    upper.ceil() as u32
}

struct FreshWriter<'a> {
    writer: &'a mut dyn Write,
    cancel: &'a PdfCancellation,
    limit: u64,
    count: u64,
    hash: blake3::Hasher,
    offsets: Vec<u64>,
    next_object: u32,
}
impl<'a> FreshWriter<'a> {
    fn new(
        writer: &'a mut dyn Write,
        limit: u64,
        objects: u32,
        cancel: &'a PdfCancellation,
    ) -> RasterRedactionResult<Self> {
        let mut offsets = Vec::new();
        offsets
            .try_reserve_exact(objects as usize + 1)
            .map_err(|_| PdfError::Allocation)?;
        offsets.resize(objects as usize + 1, 0);
        Ok(Self {
            writer,
            cancel,
            limit,
            count: 0,
            hash: blake3::Hasher::new(),
            offsets,
            next_object: 1,
        })
    }
    fn bytes(&mut self, bytes: &[u8]) -> RasterRedactionResult<()> {
        self.cancel.check()?;
        let next = self
            .count
            .checked_add(bytes.len() as u64)
            .ok_or(RasterRedactionError::Limit)?;
        if next > self.limit {
            return Err(RasterRedactionError::Limit);
        }
        self.writer
            .write_all(bytes)
            .map_err(|error| PdfError::Io(error.kind()))?;
        self.hash.update(bytes);
        self.count = next;
        Ok(())
    }
    fn text(&mut self, text: std::fmt::Arguments<'_>) -> RasterRedactionResult<()> {
        // All call sites contain only bounded numeric formatting/literal text.
        // write_fmt cannot expose an arbitrary original PDF string.
        struct Adapter<'a, 'b>(&'a mut FreshWriter<'b>, Option<RasterRedactionError>);
        impl std::fmt::Write for Adapter<'_, '_> {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                match self.0.bytes(value.as_bytes()) {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        self.1 = Some(error);
                        Err(std::fmt::Error)
                    }
                }
            }
        }
        let mut adapter = Adapter(self, None);
        let result = std::fmt::write(&mut adapter, text);
        if let Some(error) = adapter.1 {
            return Err(error);
        }
        result.map_err(|_| RasterRedactionError::Invalid)
    }
    fn begin(&mut self, object: u32) -> RasterRedactionResult<()> {
        if object != self.next_object || object as usize >= self.offsets.len() {
            return Err(RasterRedactionError::Invalid);
        }
        self.offsets[object as usize] = self.count;
        self.next_object += 1;
        self.text(format_args!("{object} 0 obj\n"))
    }
    fn finish(mut self) -> RasterRedactionResult<(u64, String)> {
        if self.next_object as usize != self.offsets.len() {
            return Err(RasterRedactionError::Invalid);
        }
        let xref = self.count;
        let size = self.offsets.len();
        self.text(format_args!("xref\n0 {size}\n0000000000 65535 f \n"))?;
        for index in 1..size {
            let offset = self.offsets[index];
            if offset > 9_999_999_999 {
                return Err(RasterRedactionError::Limit);
            }
            self.text(format_args!("{offset:010} 00000 n \n"))?;
        }
        self.text(format_args!(
            "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"
        ))?;
        self.writer
            .flush()
            .map_err(|error| PdfError::Io(error.kind()))?;
        self.cancel.check()?;
        Ok((self.count, self.hash.finalize().to_hex().to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    #[test]
    fn fractional_redaction_replaces_samples_across_tile_seams() {
        let cancel = PdfCancellation::default();
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 511.75,
                top: 0.25,
                right: 512.25,
                bottom: 1.25,
            },
        }];
        let mut left = vec![90; 512 * 2 * 4];
        let mut right = vec![90; 2 * 2 * 4];
        replace_pixels(&mut left, 512, 2, 0, 0, 0, 1.0, &regions, &cancel).unwrap();
        replace_pixels(&mut right, 2, 2, 512, 0, 0, 1.0, &regions, &cancel).unwrap();
        for y in 0..2 {
            assert_eq!(
                &left[(y * 512 + 511) * 4..(y * 512 + 512) * 4],
                [0, 0, 0, 255]
            );
            assert_eq!(&right[y * 8..y * 8 + 4], [0, 0, 0, 255]);
            assert_eq!(&right[y * 8 + 4..y * 8 + 8], [90, 90, 90, 90]);
        }
    }
    #[test]
    fn replacement_keeps_other_pages_and_outside_bytes_unchanged() {
        let mut pixels = vec![123; 4 * 4 * 4];
        let regions = [PageRedaction {
            page: 1,
            region: PdfRect {
                left: 0.0,
                top: 0.0,
                right: 4.0,
                bottom: 4.0,
            },
        }];
        replace_pixels(
            &mut pixels,
            4,
            4,
            0,
            0,
            0,
            1.0,
            &regions,
            &PdfCancellation::default(),
        )
        .unwrap();
        assert_eq!(pixels, vec![123; 64]);
    }
    #[test]
    fn cancellation_precedes_pixel_replacement() {
        let cancel = PdfCancellation::default();
        cancel.cancel();
        let mut pixels = vec![123; 16];
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 0.0,
                top: 0.0,
                right: 2.0,
                bottom: 2.0,
            },
        }];
        assert!(matches!(
            replace_pixels(&mut pixels, 2, 2, 0, 0, 0, 1.0, &regions, &cancel),
            Err(RasterRedactionError::Pdf(PdfError::Cancelled))
        ));
        assert_eq!(pixels, vec![123; 16]);
    }
    #[test]
    fn exact_edges_have_no_halo_and_underflow_cannot_leave_a_secret_sample() {
        assert_eq!((pixel_lower(1.0, 2.0), pixel_upper(1.0, 2.0)), (2, 2));
        let mut pixels = vec![123; 16];
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 0.0,
                top: 0.0,
                right: f64::from_bits(1),
                bottom: f64::from_bits(1),
            },
        }];
        replace_pixels(
            &mut pixels,
            2,
            2,
            0,
            0,
            0,
            0.25,
            &regions,
            &PdfCancellation::default(),
        )
        .unwrap();
        assert_eq!(&pixels[..4], [0, 0, 0, 255]);
        assert_eq!(&pixels[4..], &[123; 12]);
    }
    #[test]
    fn coverage_uses_the_actual_f32_scale_at_integer_boundaries() {
        let scale = effective_scale(1.2).unwrap();
        assert_eq!(scale, f64::from(1.2_f32));
        assert!(scale > 1.2);
        assert_eq!(pixel_upper(5.0, scale), 7);
        let mut pixels = vec![123; 8 * 4];
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 0.0,
                top: 0.0,
                right: 5.0,
                bottom: 1.0,
            },
        }];
        replace_pixels(
            &mut pixels,
            8,
            1,
            0,
            0,
            0,
            scale,
            &regions,
            &PdfCancellation::default(),
        )
        .unwrap();
        assert_eq!(&pixels[6 * 4..7 * 4], &[0, 0, 0, 255]);
        assert_eq!(&pixels[7 * 4..], &[123; 4]);
        let down = effective_scale(1.4).unwrap();
        assert!(down < 1.4);
        assert_eq!(pixel_lower(5.0, down), 6);
    }
    #[test]
    fn native_scale_rounding_covers_both_sides_of_tile_seams() {
        let cancel = PdfCancellation::default();
        // 1280*requested1.2 ==1536, but the actual matrix crosses into1536.
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 1279.0,
                top: 0.0,
                right: 1280.0,
                bottom: 1.0,
            },
        }];
        let mut next = vec![123; 8];
        replace_pixels(
            &mut next,
            2,
            1,
            1536,
            0,
            0,
            effective_scale(1.2).unwrap(),
            &regions,
            &cancel,
        )
        .unwrap();
        assert_eq!(&next[..4], &[0, 0, 0, 255]);
        assert_eq!(&next[4..], &[123; 4]);
        // 2560*requested1.4 ==3584, but the actual matrix starts in3583.
        let regions = [PageRedaction {
            page: 0,
            region: PdfRect {
                left: 2560.0,
                top: 0.0,
                right: 2561.0,
                bottom: 1.0,
            },
        }];
        let mut previous = vec![123; 512 * 4];
        replace_pixels(
            &mut previous,
            512,
            1,
            3072,
            0,
            0,
            effective_scale(1.4).unwrap(),
            &regions,
            &cancel,
        )
        .unwrap();
        assert_eq!(&previous[511 * 4..], &[0, 0, 0, 255]);
        assert_eq!(&previous[510 * 4..511 * 4], &[123; 4]);
    }
    #[test]
    fn every_admitted_tile_origin_has_exact_native_integer_translation() {
        for requested in [0.25, 0.3, 1.2, 1.4, 7.999999, 8.0] {
            let scale = effective_scale(requested).unwrap();
            for pixel in (0..524_280).step_by(512) {
                let origin = tile_origin(pixel, scale).unwrap();
                assert_eq!(f64::from((-origin * scale) as f32), -f64::from(pixel));
            }
        }
    }
    #[test]
    fn overlapping_rectangles_are_each_charged_before_any_pixel_allocation() {
        let geometry =
            PageGeometry::new([0.0, 0.0, 1000.0, 1000.0], 0, PdfLimits::default()).unwrap();
        let pages = [RasterPage {
            geometry,
            width: 1000,
            height: 1000,
            columns: 2,
            rows: 2,
            page_object: 3,
        }];
        let regions = vec![
            PageRedaction {
                page: 0,
                region: PdfRect {
                    left: 0.0,
                    top: 0.0,
                    right: 1000.0,
                    bottom: 1000.0
                }
            };
            4096
        ];
        let work =
            admit_raster_redaction(&[[1000.0, 1000.0]], 1.0, 4096, PdfLimits::default()).unwrap();
        assert!(matches!(
            admit_regions(&pages, &regions, 1.0, work, &PdfCancellation::default()),
            Err(RasterRedactionError::Limit)
        ));
    }
    #[test]
    fn writer_bounds_and_object_sequence_refuse_without_success_receipt() {
        let cancel = PdfCancellation::default();
        let mut bytes = Vec::new();
        let mut writer = FreshWriter::new(&mut bytes, 4, 1, &cancel).unwrap();
        assert!(matches!(
            writer.begin(2),
            Err(RasterRedactionError::Invalid)
        ));
        assert!(matches!(
            writer.bytes(b"12345"),
            Err(RasterRedactionError::Limit)
        ));
        drop(writer);
        assert!(bytes.is_empty());
    }
    #[test]
    fn fresh_syntax_has_only_owned_objects_and_correct_xref_offsets() {
        let cancel = PdfCancellation::default();
        let mut bytes = Vec::new();
        let mut writer = FreshWriter::new(&mut bytes, 1000, 2, &cancel).unwrap();
        writer.bytes(b"%PDF-1.7\n").unwrap();
        writer.begin(1).unwrap();
        writer
            .bytes(b"<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")
            .unwrap();
        writer.begin(2).unwrap();
        writer
            .bytes(b"<< /Type /Pages /Count 0 /Kids [] >>\nendobj\n")
            .unwrap();
        let (length, hash) = writer.finish().unwrap();
        assert_eq!(length, bytes.len() as u64);
        assert_eq!(hash, blake3::hash(&bytes).to_hex().to_string());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("0000000009 00000 n"));
        for forbidden in [
            "/Info",
            "/Metadata",
            "/Font",
            "/JavaScript",
            "/Names",
            "/EmbeddedFiles",
            "/Prev",
            "/Encrypt",
        ] {
            assert!(!text.contains(forbidden));
        }
    }
}

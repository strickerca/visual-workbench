//! Fresh PC reconstruction. Unaffected admitted polygons remain vector; source
//! objects/dictionaries/resources are never copied to the generated document.
use crate::{PageGeometry, PageRedaction, PdfCancellation, PdfDocument, PdfError, PdfNotice, PdfPoint, PdfRect, PdfTileRequest, RasterSecurityPolicy};
use std::io::Write;

#[derive(Debug, thiserror::Error)]
pub enum PcRedactionError {
    #[error(transparent)] Pdf(#[from] PdfError),
    #[error("invalid PC redaction request")] Invalid,
    #[error("PC redaction size or work limit exceeded")] Limit,
    #[error("PDF content is outside the admitted PC subset; explicitly choose full-page raster redaction")] UnsupportedContent,
    #[error("unaffected PDF object {object} on page {page} is unsupported; explicitly choose full-page raster redaction")] UnsupportedObject { page: u32, object: u32 },
    #[error("redaction source or prepared geometry changed")] SourceMismatch,
    #[error("PC redaction verification failed")] Verification,
    #[error("verified qpdf runtime unavailable or incompatible")] CleanupUnavailable,
    #[error("qpdf cleanup refused the private generated document")] Cleanup,
}
pub type PcRedactionResult<T> = Result<T, PcRedactionError>;
const TILE: u32 = 512;
const MAX_REGIONS: usize = 4096;
const MAX_PIXELS: u64 = 100_000_000;
const MAX_OBJECTS: u32 = 100_000;
const MAX_COMPARISONS: u64 = 10_000_000;
pub(crate) const MAX_POINTS: usize = 250_000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PcRedactionOptions { pub pixels_per_point: f64, pub security: RasterSecurityPolicy }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcRedactionWork { pub pages: u32, pub removed_objects: u32, pub preserved_paths: u32, pub tiles: u32, pub raster_pixels: u64, pub maximum_encoded_bytes: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcRedactionProgress { pub pages_complete: u32, pub tiles_complete: u32, pub total_tiles: u32 }

/// Opaque proof made only by the verified qpdf source-admission path. It binds
/// the exact source, not an assertion received from a peer or an arbitrary DTO.
pub struct PcSourceAdmission { pub(crate) source_blake3: String, pub(crate) pages: u32 }
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PathPoint { pub point: PdfPoint, pub move_to: bool, pub close: bool }
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PlainPath { pub points: Vec<PathPoint>, pub rgb: [u8; 3], pub even_odd: bool }
pub(crate) struct PaintProbe { pub bounds: PdfRect, pub path: Option<PlainPath>, pub object: u32 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RasterTile { pub x: u32, pub y: u32, pub width: u32, pub height: u32 }
pub(crate) struct PcPage { pub geometry: PageGeometry, pub paths: Vec<PlainPath>, pub tiles: Vec<RasterTile>, pub object: u32 }
pub struct PcRedactionPlan {
    pub(crate) source_blake3: String,
    pub(crate) plan_blake3: String,
    pub(crate) pages: Vec<PcPage>,
    regions: Vec<PageRedaction>,
    requested_scale: f64,
    pub(crate) scale: f64,
    work: PcRedactionWork,
    additional_memory: u64,
}
pub struct UnverifiedPcCopy {
    pub(crate) source_blake3: String, pub(crate) plan_blake3: String,
    pub(crate) output_blake3: String, pub(crate) bytes: u64,
    pub(crate) tile_hashes: Vec<[u8; 32]>,
}
impl UnverifiedPcCopy {
    pub fn output_blake3(&self) -> &str { &self.output_blake3 }
    pub fn bytes(&self) -> u64 { self.bytes }
}
/// Only the actual qpdf rewrite + post-write structural/payload audit makes
/// this token. It has no public DTO constructor and cannot be peer-supplied.
pub struct CleanedPcCopy {
    pub(crate) source_blake3: String, pub(crate) plan_blake3: String,
    pub(crate) output_blake3: String, pub(crate) bytes: u64,
}
#[derive(Clone, Debug, PartialEq)]
pub struct VerifiedPcRedaction {
    source_blake3: String, plan_blake3: String, output_blake3: String,
    work: PcRedactionWork, regions: u32, bytes: u64, requested_scale: f64, scale: f64,
}
impl VerifiedPcRedaction {
    pub fn source_blake3(&self) -> &str { &self.source_blake3 }
    pub fn plan_blake3(&self) -> &str { &self.plan_blake3 }
    pub fn output_blake3(&self) -> &str { &self.output_blake3 }
    pub fn work(&self) -> PcRedactionWork { self.work }
    pub fn regions(&self) -> u32 { self.regions }
    pub fn bytes(&self) -> u64 { self.bytes }
    pub fn requested_pixels_per_point(&self) -> f64 { self.requested_scale }
    pub fn pixels_per_point(&self) -> f64 { self.scale }
}

impl PcRedactionPlan {
    pub fn prepare(document: &mut PdfDocument<'_>, admission: PcSourceAdmission, regions: Vec<PageRedaction>, options: PcRedactionOptions, cancel: &PdfCancellation) -> PcRedactionResult<Self> {
        cancel.check()?;
        if regions.is_empty() || regions.len() > MAX_REGIONS || !options.pixels_per_point.is_finite() || !(0.25..=8.0).contains(&options.pixels_per_point) { return Err(PcRedactionError::Invalid); }
        if admission.source_blake3 != document.info().source_blake3 || admission.pages != document.info().pages { return Err(PcRedactionError::SourceMismatch); }
        if document.info().encrypted && options.security != RasterSecurityPolicy::ExplicitlyRemoveEncryption { return Err(PdfError::EncryptedExport.into()); }
        if document.info().notices.contains(&PdfNotice::NonInteractiveForms) { return Err(PdfError::UnsupportedForms.into()); }
        let limits = document.redaction_limits();
        let scale = f64::from(options.pixels_per_point as f32);
        let base = (regions.capacity() as u64).checked_mul(std::mem::size_of::<PageRedaction>() as u64)
            .and_then(|n| n.checked_add(u64::from(admission.pages) * std::mem::size_of::<PcPage>() as u64)).ok_or(PcRedactionError::Limit)?;
        document.admit_redaction_memory(base)?;
        let mut pages = Vec::new(); pages.try_reserve_exact(admission.pages as usize).map_err(|_| PdfError::Allocation)?;
        let mut work = PcRedactionWork { pages: admission.pages, removed_objects: 0, preserved_paths: 0, tiles: 0, raster_pixels: 0, maximum_encoded_bytes: 0 };
        let mut additional_memory = base;
        let mut points_remaining = MAX_POINTS;
        let mut object = 3u32;
        for index in 0..admission.pages {
            cancel.check()?;
            let info = document.page_info(index, cancel)?;
            if info.annotations != 0 { return Err(PcRedactionError::UnsupportedContent); }
            let mut area = Vec::new(); area.try_reserve_exact(regions.len()).map_err(|_| PdfError::Allocation)?;
            area.extend(regions.iter().filter(|region| region.page == index).map(|region| region.region));
            let (width, height) = dimensions(info.geometry, scale)?;
            for region in &area {
                region.validate()?;
                if region.left < 0.0 || region.top < 0.0 || region.right > info.geometry.width() || region.bottom > info.geometry.height()
                    || region.left >= region.right || region.top >= region.bottom { return Err(PcRedactionError::Invalid); }
            }
            let grid_count = width.div_ceil(TILE).checked_mul(height.div_ceil(TILE)).ok_or(PcRedactionError::Limit)?;
            let probe_memory = u64::from(info.objects).checked_mul(192).and_then(|n| n.checked_add(points_remaining as u64 * 2 * std::mem::size_of::<PathPoint>() as u64))
                .and_then(|n| n.checked_add(u64::from(grid_count) * 32)).and_then(|n| n.checked_add(area.capacity() as u64 * 32)).ok_or(PcRedactionError::Limit)?;
            document.admit_redaction_memory(additional_memory.checked_add(probe_memory).ok_or(PcRedactionError::Limit)?)?;
            let probes = document.pc_paint_probes(index, &mut points_remaining, cancel)?;
            let (paths, tiles, removed) = partition(index, info.geometry, width, height, scale, probes, &area, cancel)?;
            if paths.len() as u64 + tiles.len() as u64 > u64::from(limits.objects_per_page) { return Err(PcRedactionError::Limit); }
            let page_points: usize = paths.iter().map(|path| path.points.capacity()).sum();
            additional_memory = additional_memory.checked_add(page_points as u64 * std::mem::size_of::<PathPoint>() as u64)
                .and_then(|n| n.checked_add(paths.capacity() as u64 * std::mem::size_of::<PlainPath>() as u64))
                .and_then(|n| n.checked_add(tiles.capacity() as u64 * std::mem::size_of::<RasterTile>() as u64)).ok_or(PcRedactionError::Limit)?;
            work.removed_objects = work.removed_objects.checked_add(removed).ok_or(PcRedactionError::Limit)?;
            work.preserved_paths = work.preserved_paths.checked_add(paths.len() as u32).ok_or(PcRedactionError::Limit)?;
            work.tiles = work.tiles.checked_add(tiles.len() as u32).ok_or(PcRedactionError::Limit)?;
            for tile in &tiles { origin(tile.x, scale)?; origin(tile.y, scale)?; work.raster_pixels = work.raster_pixels.checked_add(u64::from(tile.width) * u64::from(tile.height)).ok_or(PcRedactionError::Limit)?; }
            object = object.checked_add(2).and_then(|n| n.checked_add(tiles.len() as u32)).ok_or(PcRedactionError::Limit)?;
            if object > MAX_OBJECTS + 1 || work.raster_pixels > MAX_PIXELS { return Err(PcRedactionError::Limit); }
            let page_object = object - 2 - tiles.len() as u32;
            pages.push(PcPage { geometry: info.geometry, paths, tiles, object: page_object });
        }
        if regions.iter().any(|region| region.page >= admission.pages) { return Err(PcRedactionError::Invalid); }
        // Numeric formatting is fixed decimal and bounded; this reserves xref,
        // page dictionaries, image headers, content matrices and all path data.
        work.maximum_encoded_bytes = work.raster_pixels.checked_mul(3)
            .and_then(|n| n.checked_add(u64::from(work.tiles) * 512))
            .and_then(|n| n.checked_add(u64::from(work.preserved_paths) * 256))
            .and_then(|n| n.checked_add((MAX_POINTS - points_remaining) as u64 * 96))
            .and_then(|n| n.checked_add(u64::from(work.pages) * 4096 + 65536)).ok_or(PcRedactionError::Limit)?;
        if work.maximum_encoded_bytes > limits.output_bytes.min(limits.input_bytes) { return Err(PcRedactionError::Limit); }
        additional_memory = additional_memory.checked_add(u64::from(TILE) * u64::from(TILE) * 7)
            .and_then(|n| n.checked_add(u64::from(object) * 8 + u64::from(work.tiles) * 32))
            .and_then(|n| n.checked_add((MAX_POINTS - points_remaining) as u64 * 96 + u64::from(work.tiles) * 128)).ok_or(PcRedactionError::Limit)?;
        document.admit_redaction_memory(additional_memory)?;
        limits.admit_memory(work.maximum_encoded_bytes, additional_memory)?;
        // The qpdf post-rewrite audit may retain one decoded/generated content
        // pair. Refuse this predictable verification budget before any output.
        limits.admit_memory(work.maximum_encoded_bytes, additional_memory.checked_add(64 * 1024 * 1024).ok_or(PcRedactionError::Limit)?)?;
        let source_blake3 = admission.source_blake3;
        let mut hash = blake3::Hasher::new(); hash.update(b"vw-pc-path-redaction-v1\0"); hash.update(source_blake3.as_bytes());
        hash.update(&options.pixels_per_point.to_bits().to_le_bytes()); hash.update(&scale.to_bits().to_le_bytes());
        hash.update(&[u8::from(options.security == RasterSecurityPolicy::ExplicitlyRemoveEncryption)]);
        for page in &pages {
            for value in [page.geometry.width(), page.geometry.height()] { hash.update(&value.to_bits().to_le_bytes()); }
            for tile in &page.tiles { for value in [tile.x, tile.y, tile.width, tile.height] { hash.update(&value.to_le_bytes()); } }
            for path in &page.paths { hash.update(&path.rgb); hash.update(&[u8::from(path.even_odd)]); for point in &path.points { hash.update(&point.point.x.to_bits().to_le_bytes()); hash.update(&point.point.y.to_bits().to_le_bytes()); hash.update(&[u8::from(point.move_to), u8::from(point.close)]); } }
        }
        for region in &regions { hash.update(&region.page.to_le_bytes()); for value in [region.region.left, region.region.top, region.region.right, region.region.bottom] { hash.update(&value.to_bits().to_le_bytes()); } }
        let plan_blake3 = hash.finalize().to_hex().to_string();
        Ok(Self { source_blake3, plan_blake3, pages, regions, requested_scale: options.pixels_per_point, scale, work, additional_memory })
    }
    pub fn work(&self) -> PcRedactionWork { self.work }
    pub fn plan_blake3(&self) -> &str { &self.plan_blake3 }
    pub fn requested_pixels_per_point(&self) -> f64 { self.requested_scale }
    pub fn pixels_per_point(&self) -> f64 { self.scale }

    pub fn write_unverified(&self, document: &mut PdfDocument<'_>, writer: &mut dyn Write, cancel: &PdfCancellation, progress: &mut dyn FnMut(PcRedactionProgress)) -> PcRedactionResult<UnverifiedPcCopy> {
        self.check_source(document, cancel)?;
        document.admit_redaction_memory(self.additional_memory)?;
        let objects = self.work.pages.checked_mul(2).and_then(|n| n.checked_add(self.work.tiles + 2)).ok_or(PcRedactionError::Limit)?;
        let mut output = FreshWriter::new(writer, self.work.maximum_encoded_bytes, objects, cancel)?;
        output.bytes(b"%PDF-1.7\n%VW fresh path reconstruction\n")?;
        output.begin(1)?; output.bytes(b"<< /Type /Catalog /Pages 2 0 R >>\nendobj\n")?;
        output.begin(2)?; output.text(format_args!("<< /Type /Pages /Count {} /Kids [", self.work.pages))?;
        for page in &self.pages { output.text(format_args!("{} 0 R ", page.object))?; }
        output.bytes(b"] >>\nendobj\n")?;
        let mut tile_hashes = Vec::new(); tile_hashes.try_reserve_exact(self.work.tiles as usize).map_err(|_| PdfError::Allocation)?;
        let mut complete = 0u32;
        for (index, page) in self.pages.iter().enumerate() {
            let content = self.page_content(page)?;
            output.begin(page.object)?;
            output.text(format_args!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.9} {:.9}] /Resources << /XObject << ", page.geometry.width(), page.geometry.height()))?;
            for tile in 0..page.tiles.len() { output.text(format_args!("/T{tile} {} 0 R ", page.object + 2 + tile as u32))?; }
            output.text(format_args!(">> >> /Contents {} 0 R >>\nendobj\n", page.object + 1))?;
            output.begin(page.object + 1)?; output.text(format_args!("<< /Length {} >>\nstream\n", content.len()))?; output.bytes(&content)?; output.bytes(b"endstream\nendobj\n")?;
            drop(content);
            for (ordinal, tile) in page.tiles.iter().copied().enumerate() {
                cancel.check()?;
                let mut pixels = document.render_tile(index as u32, PdfTileRequest { x: origin(tile.x, self.scale)?, y: origin(tile.y, self.scale)?, pixels_per_point: self.scale, width: tile.width, height: tile.height, include_annotations: false }, cancel)?.rgba;
                sanitize(&mut pixels, tile, index as u32, self.scale, &self.regions, cancel)?;
                let mut rgb = Vec::new(); rgb.try_reserve_exact(tile.width as usize * tile.height as usize * 3).map_err(|_| PdfError::Allocation)?;
                for pixel in pixels.chunks_exact(4) { if pixel[3] != 255 { return Err(PcRedactionError::Verification); } rgb.extend_from_slice(&pixel[..3]); }
                tile_hashes.push(*blake3::hash(&rgb).as_bytes());
                output.begin(page.object + 2 + ordinal as u32)?;
                output.text(format_args!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n", tile.width, tile.height, rgb.len()))?;
                output.bytes(&rgb)?; output.bytes(b"\nendstream\nendobj\n")?;
                complete += 1; progress(PcRedactionProgress { pages_complete: index as u32, tiles_complete: complete, total_tiles: self.work.tiles });
            }
            progress(PcRedactionProgress { pages_complete: index as u32 + 1, tiles_complete: complete, total_tiles: self.work.tiles });
        }
        let (bytes, output_blake3) = output.finish()?;
        Ok(UnverifiedPcCopy { source_blake3: self.source_blake3.clone(), plan_blake3: self.plan_blake3.clone(), output_blake3, bytes, tile_hashes })
    }

    /// After qpdf cleanup, reopen its exact output with the independent PDFium
    /// runtime. Only this method makes a verified redaction receipt.
    pub fn verify(&self, document: &mut PdfDocument<'_>, cleaned: CleanedPcCopy, cancel: &PdfCancellation) -> PcRedactionResult<VerifiedPcRedaction> {
        if cleaned.source_blake3 != self.source_blake3 || cleaned.plan_blake3 != self.plan_blake3 || cleaned.output_blake3 != document.info().source_blake3
            || document.info().pages != self.work.pages || document.info().encrypted || document.info().notices.contains(&PdfNotice::NonInteractiveForms)
        { return Err(PcRedactionError::Verification); }
        document.admit_redaction_memory(self.additional_memory)?;
        for (index, page) in self.pages.iter().enumerate() {
            cancel.check()?;
            let info = document.page_info(index as u32, cancel)?;
            if (info.geometry.width() - page.geometry.width()).abs() > 0.0001 || (info.geometry.height() - page.geometry.height()).abs() > 0.0001 { return Err(PcRedactionError::Verification); }
            document.verify_pc_page(index as u32, &page.paths, page.tiles.len() as u32, cancel)?;
        }
        cancel.check()?;
        Ok(VerifiedPcRedaction { source_blake3: self.source_blake3.clone(), plan_blake3: self.plan_blake3.clone(), output_blake3: cleaned.output_blake3,
            work: self.work, regions: self.regions.len() as u32, bytes: cleaned.bytes, requested_scale: self.requested_scale, scale: self.scale })
    }
    pub(crate) fn page_content(&self, page: &PcPage) -> PcRedactionResult<Vec<u8>> {
        let points: usize = page.paths.iter().map(|path| path.points.len()).sum();
        let maximum = points.checked_mul(96).and_then(|n| n.checked_add(page.paths.len() * 256 + page.tiles.len() * 128)).ok_or(PcRedactionError::Limit)?;
        let mut bytes = Vec::new(); bytes.try_reserve_exact(maximum).map_err(|_| PdfError::Allocation)?;
        for path in &page.paths {
            writeln!(&mut bytes, "q {:.9} {:.9} {:.9} rg", f64::from(path.rgb[0]) / 255.0, f64::from(path.rgb[1]) / 255.0, f64::from(path.rgb[2]) / 255.0).map_err(|e| PdfError::Io(e.kind()))?;
            for point in &path.points {
                writeln!(&mut bytes, "{:.9} {:.9} {}", point.point.x, page.geometry.height() - point.point.y, if point.move_to { "m" } else { "l" }).map_err(|e| PdfError::Io(e.kind()))?;
                if point.close { bytes.extend_from_slice(b"h\n"); }
            }
            bytes.extend_from_slice(if path.even_odd { b"f* Q\n" } else { b"f Q\n" });
        }
        for (index, tile) in page.tiles.iter().enumerate() {
            writeln!(&mut bytes, "q {:.9} 0 0 {:.9} {:.9} {:.9} cm /T{index} Do Q", f64::from(tile.width) / self.scale, f64::from(tile.height) / self.scale,
                f64::from(tile.x) / self.scale, page.geometry.height() - f64::from(tile.y + tile.height) / self.scale).map_err(|e| PdfError::Io(e.kind()))?;
        }
        if bytes.len() > maximum { return Err(PcRedactionError::Limit); }
        Ok(bytes)
    }
    fn check_source(&self, document: &mut PdfDocument<'_>, cancel: &PdfCancellation) -> PcRedactionResult<()> {
        if document.info().source_blake3 != self.source_blake3 || document.info().pages != self.work.pages { return Err(PcRedactionError::SourceMismatch); }
        for (index, page) in self.pages.iter().enumerate() { if document.page_info(index as u32, cancel)?.geometry != page.geometry { return Err(PcRedactionError::SourceMismatch); } }
        cancel.check()?; Ok(())
    }
}

fn dimensions(geometry: PageGeometry, scale: f64) -> PcRedactionResult<(u32, u32)> {
    Ok((upper(geometry.width(), scale), upper(geometry.height(), scale)))
}
fn origin(pixel: u32, scale: f64) -> PcRedactionResult<f64> {
    let value = f64::from(pixel) / scale;
    if f64::from((-value * scale) as f32) != -f64::from(pixel) { return Err(PcRedactionError::Invalid); }
    Ok(value)
}
#[allow(clippy::too_many_arguments)]
fn partition(page: u32, geometry: PageGeometry, width: u32, height: u32, scale: f64, probes: Vec<PaintProbe>, regions: &[PdfRect], cancel: &PdfCancellation) -> PcRedactionResult<(Vec<PlainPath>, Vec<RasterTile>, u32)> {
    let columns = width.div_ceil(TILE); let rows = height.div_ceil(TILE);
    let count = columns.checked_mul(rows).ok_or(PcRedactionError::Limit)? as usize;
    if count > MAX_OBJECTS as usize { return Err(PcRedactionError::Limit); }
    let mut replaced = Vec::new(); replaced.try_reserve_exact(count).map_err(|_| PdfError::Allocation)?; replaced.resize(count, false);
    let mut removed = Vec::new(); removed.try_reserve_exact(probes.len()).map_err(|_| PdfError::Allocation)?; removed.resize(probes.len(), false);
    let mut comparisons = 0u64;
    for region in regions { mark_tiles(*region, width, height, scale, &mut replaced)?; }
    loop {
        let mut changed = false;
        for (index, probe) in probes.iter().enumerate() {
            cancel.check()?;
            if removed[index] { continue; }
            // Two display pixels conservatively include antialiased boundary
            // contributions; complete objects crossing tile seams are removed.
            let bounds = expand(probe.bounds, geometry, 2.0 / scale);
            if bounds.left >= bounds.right || bounds.top >= bounds.bottom { removed[index] = true; continue; }
            let range = tile_range(bounds, width, height, scale)?;
            let mut touches = false;
            for row in range.1..range.3 { for column in range.0..range.2 {
                comparisons = comparisons.checked_add(1).ok_or(PcRedactionError::Limit)?;
                if comparisons > MAX_COMPARISONS { return Err(PcRedactionError::Limit); }
                touches |= replaced[(row * columns + column) as usize];
            }}
            if touches { removed[index] = true; changed |= mark_tiles(bounds, width, height, scale, &mut replaced)?; }
        }
        if !changed { break; }
    }
    let mut paths = Vec::new(); paths.try_reserve_exact(probes.len()).map_err(|_| PdfError::Allocation)?;
    let mut removed_count = 0u32;
    for (probe, removed) in probes.into_iter().zip(removed) {
        if removed { removed_count = removed_count.checked_add(1).ok_or(PcRedactionError::Limit)?; }
        else { paths.push(probe.path.ok_or(PcRedactionError::UnsupportedObject { page, object: probe.object })?); }
    }
    let mut tiles = Vec::new(); tiles.try_reserve_exact(replaced.iter().filter(|b| **b).count()).map_err(|_| PdfError::Allocation)?;
    for row in 0..rows { for column in 0..columns { if replaced[(row * columns + column) as usize] {
        let x = column * TILE; let y = row * TILE; tiles.push(RasterTile { x, y, width: TILE.min(width - x), height: TILE.min(height - y) });
    }}}
    Ok((paths, tiles, removed_count))
}
fn expand(bounds: PdfRect, geometry: PageGeometry, margin: f64) -> PdfRect {
    PdfRect { left: (bounds.left - margin).max(0.0), top: (bounds.top - margin).max(0.0), right: (bounds.right + margin).min(geometry.width()), bottom: (bounds.bottom + margin).min(geometry.height()) }
}
fn tile_range(bounds: PdfRect, width: u32, height: u32, scale: f64) -> PcRedactionResult<(u32, u32, u32, u32)> {
    if bounds.left < 0.0 || bounds.top < 0.0 || bounds.left >= bounds.right || bounds.top >= bounds.bottom { return Err(PcRedactionError::Invalid); }
    Ok((lower(bounds.left, scale).min(width) / TILE, lower(bounds.top, scale).min(height) / TILE,
        upper(bounds.right, scale).min(width).div_ceil(TILE), upper(bounds.bottom, scale).min(height).div_ceil(TILE)))
}
fn mark_tiles(bounds: PdfRect, width: u32, height: u32, scale: f64, replaced: &mut [bool]) -> PcRedactionResult<bool> {
    let columns = width.div_ceil(TILE); let range = tile_range(bounds, width, height, scale)?; let mut changed = false;
    for row in range.1..range.3 { for column in range.0..range.2 {
        let value = replaced.get_mut((row * columns + column) as usize).ok_or(PcRedactionError::Invalid)?; changed |= !*value; *value = true;
    }}
    Ok(changed)
}
fn sanitize(pixels: &mut [u8], tile: RasterTile, page: u32, scale: f64, regions: &[PageRedaction], cancel: &PdfCancellation) -> PcRedactionResult<()> {
    if pixels.len() != tile.width as usize * tile.height as usize * 4 { return Err(PcRedactionError::Verification); }
    for region in regions.iter().filter(|region| region.page == page) {
        let x0 = lower(region.region.left, scale).max(tile.x).min(tile.x + tile.width) - tile.x;
        let x1 = upper(region.region.right, scale).max(tile.x).min(tile.x + tile.width) - tile.x;
        let y0 = lower(region.region.top, scale).max(tile.y).min(tile.y + tile.height) - tile.y;
        let y1 = upper(region.region.bottom, scale).max(tile.y).min(tile.y + tile.height) - tile.y;
        for y in y0..y1 { cancel.check()?; for x in x0..x1 { let start = ((y * tile.width + x) * 4) as usize; pixels[start..start + 4].copy_from_slice(&[0, 0, 0, 255]); } }
    }
    cancel.check()?; Ok(())
}
fn lower(value: f64, scale: f64) -> u32 {
    let product = value * scale; let residual = value.mul_add(scale, -product);
    let value = if residual < 0.0 && product > 0.0 { f64::from_bits(product.to_bits() - 1) } else { product }; value.floor() as u32
}
fn upper(value: f64, scale: f64) -> u32 {
    let product = value * scale; if value > 0.0 && product == 0.0 { return 1; }
    let value = if value.mul_add(scale, -product) > 0.0 { f64::from_bits(product.to_bits() + 1) } else { product }; value.ceil() as u32
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
    fn new(writer: &'a mut dyn Write, limit: u64, objects: u32, cancel: &'a PdfCancellation) -> PcRedactionResult<Self> {
        let mut offsets = Vec::new(); offsets.try_reserve_exact(objects as usize + 1).map_err(|_| PdfError::Allocation)?; offsets.resize(objects as usize + 1, 0);
        Ok(Self { writer, cancel, limit, count: 0, hash: blake3::Hasher::new(), offsets, next_object: 1 })
    }
    fn bytes(&mut self, bytes: &[u8]) -> PcRedactionResult<()> {
        self.cancel.check()?;
        let next = self.count.checked_add(bytes.len() as u64).ok_or(PcRedactionError::Limit)?;
        if next > self.limit { return Err(PcRedactionError::Limit); }
        self.writer.write_all(bytes).map_err(|error| PdfError::Io(error.kind()))?;
        self.hash.update(bytes); self.count = next;
        Ok(())
    }
    fn text(&mut self, text: std::fmt::Arguments<'_>) -> PcRedactionResult<()> {
        // All call sites contain only bounded numeric formatting/literal text.
        // write_fmt cannot expose an arbitrary original PDF string.
        struct Adapter<'a, 'b>(&'a mut FreshWriter<'b>, Option<PcRedactionError>);
        impl std::fmt::Write for Adapter<'_, '_> {
            fn write_str(&mut self, value: &str) -> std::fmt::Result {
                match self.0.bytes(value.as_bytes()) { Ok(()) => Ok(()), Err(error) => { self.1 = Some(error); Err(std::fmt::Error) } }
            }
        }
        let mut adapter = Adapter(self, None);
        let result = std::fmt::write(&mut adapter, text);
        if let Some(error) = adapter.1 { return Err(error); }
        result.map_err(|_| PcRedactionError::Invalid)
    }
    fn begin(&mut self, object: u32) -> PcRedactionResult<()> {
        if object != self.next_object || object as usize >= self.offsets.len() { return Err(PcRedactionError::Invalid); }
        self.offsets[object as usize] = self.count; self.next_object += 1;
        self.text(format_args!("{object} 0 obj\n"))
    }
    fn finish(mut self) -> PcRedactionResult<(u64, String)> {
        if self.next_object as usize != self.offsets.len() { return Err(PcRedactionError::Invalid); }
        let xref = self.count;
        let size = self.offsets.len();
        self.text(format_args!("xref\n0 {size}\n0000000000 65535 f \n"))?;
        for index in 1..size {
            let offset = self.offsets[index];
            if offset > 9_999_999_999 { return Err(PcRedactionError::Limit); }
            self.text(format_args!("{offset:010} 00000 n \n"))?;
        }
        self.text(format_args!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n"))?;
        self.writer.flush().map_err(|error| PdfError::Io(error.kind()))?;
        self.cancel.check()?;
        Ok((self.count, self.hash.finalize().to_hex().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PdfLimits;
    fn geometry() -> PageGeometry { PageGeometry::new([0.0, 0.0, 2048.0, 512.0], 0, PdfLimits::default()).unwrap() }
    fn rectangle(left: f64, right: f64) -> PdfRect { PdfRect { left, top: 10.0, right, bottom: 30.0 } }
    fn polygon() -> PlainPath {
        PlainPath { points: vec![PathPoint { point: PdfPoint { x: 1700.0, y: 10.0 }, move_to: true, close: false },
            PathPoint { point: PdfPoint { x: 1720.0, y: 10.0 }, move_to: false, close: false },
            PathPoint { point: PdfPoint { x: 1720.0, y: 30.0 }, move_to: false, close: true }], rgb: [255, 0, 0], even_odd: false }
    }
    #[test] fn covered_text_is_removed_and_unaffected_polygon_stays_vector() {
        let probes = vec![PaintProbe { bounds: rectangle(20.0, 40.0), path: None, object: 0 }, PaintProbe { bounds: rectangle(1700.0, 1720.0), path: Some(polygon()), object: 1 }];
        let (paths, tiles, removed) = partition(0, geometry(), 2048, 512, 1.0, probes, &[rectangle(22.0, 25.0)], &PdfCancellation::default()).unwrap();
        assert_eq!(paths, vec![polygon()]); assert_eq!(removed, 1); assert_eq!(tiles, vec![RasterTile { x: 0, y: 0, width: 512, height: 512 }]);
    }
    #[test] fn removal_closure_follows_objects_across_raster_tile_boundaries() {
        let probes = vec![PaintProbe { bounds: rectangle(900.0, 1100.0), path: None, object: 0 }, PaintProbe { bounds: rectangle(500.0, 520.0), path: None, object: 1 }, PaintProbe { bounds: rectangle(1700.0, 1720.0), path: Some(polygon()), object: 2 }];
        let (paths, tiles, removed) = partition(0, geometry(), 2048, 512, 1.0, probes, &[rectangle(5.0, 8.0)], &PdfCancellation::default()).unwrap();
        assert_eq!(removed, 2); assert_eq!(tiles.iter().map(|t| t.x).collect::<Vec<_>>(), vec![0, 512, 1024]); assert_eq!(paths, vec![polygon()]);
    }
    #[test] fn unaffected_unsupported_object_has_exact_page_and_object_refusal() {
        let probes = vec![PaintProbe { bounds: rectangle(1700.0, 1720.0), path: None, object: 7 }];
        assert!(matches!(partition(5, geometry(), 2048, 512, 1.0, probes, &[rectangle(5.0, 8.0)], &PdfCancellation::default()), Err(PcRedactionError::UnsupportedObject { page: 5, object: 7 })));
    }
    #[test] fn a_page_without_regions_cannot_silently_drop_unsupported_text() {
        let probes = vec![PaintProbe { bounds: rectangle(1700.0, 1720.0), path: None, object: 2 }];
        assert!(matches!(partition(1, geometry(), 2048, 512, 1.0, probes, &[], &PdfCancellation::default()), Err(PcRedactionError::UnsupportedObject { page: 1, object: 2 })));
    }
    #[test] fn black_replacement_uses_actual_f32_scale_and_keeps_outside_samples() {
        let tile = RasterTile { x: 0, y: 0, width: 8, height: 1 };
        let mut pixels = vec![123; 32];
        let regions = [PageRedaction { page: 0, region: PdfRect { left: 0.0, top: 0.0, right: 5.0, bottom: 0.5 } }];
        sanitize(&mut pixels, tile, 0, f64::from(1.2f32), &regions, &PdfCancellation::default()).unwrap();
        assert_eq!(&pixels[..28], &[0, 0, 0, 255].repeat(7)); assert_eq!(&pixels[28..], &[123; 4]);
    }
    #[test] fn cancellation_does_not_produce_a_partition_or_replace_pixels() {
        let cancel = PdfCancellation::default(); cancel.cancel();
        let probes = vec![PaintProbe { bounds: rectangle(1700.0, 1720.0), path: Some(polygon()), object: 0 }];
        assert!(matches!(partition(0, geometry(), 2048, 512, 1.0, probes, &[rectangle(5.0, 8.0)], &cancel), Err(PcRedactionError::Pdf(PdfError::Cancelled))));
        let mut pixels = vec![123; 4];
        let regions = [PageRedaction { page: 0, region: PdfRect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 } }];
        assert!(sanitize(&mut pixels, RasterTile { x: 0, y: 0, width: 1, height: 1 }, 0, 1.0, &regions, &cancel).is_err()); assert_eq!(pixels, vec![123; 4]);
    }
}

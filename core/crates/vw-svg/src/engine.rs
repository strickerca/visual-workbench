use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};
use usvg::{FontResolver, ImageHrefResolver, Options, Tree, fontdb};

use crate::preflight::inspect;
use crate::{FidelityNotice, Limits, SvgError, SvgResult, TileRequest};

const INTER: &[u8] = include_bytes!("../../vw-raster/fonts/Inter.ttf");
const SANS: &[u8] = include_bytes!("../../vw-raster/fonts/NotoSans.ttf");
const MONO: &[u8] = include_bytes!("../../vw-raster/fonts/NotoSansMono.ttf");

/// A worker-owned static SVG. No path, URL, or system-font loader is exposed.
/// Keep this object in the isolated codec process on Windows; parser/render
/// cancellation is cooperative between calls and the parent enforces deadlines.
pub struct SvgDocument {
    pub(crate) tree: Tree,
    limits: Limits,
    workspace: u64,
    layers: u64,
    filters: u64,
    notices: Vec<FidelityNotice>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SvgInfo {
    pub width: f64,
    pub height: f64,
    pub notices: Vec<FidelityNotice>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SvgTile {
    pub width: u32,
    pub height: u32,
    /// Straight (unassociated) RGBA8, sRGB. Transparent RGB is normalized to zero.
    pub rgba: Vec<u8>,
}

impl SvgDocument {
    pub(crate) fn admit_metadata_readback(&self, bytes: usize) -> SvgResult<()> {
        let parser = self.limits.parse_memory(bytes, bytes as u64)?;
        self.limits
            .admit(self.workspace.checked_add(parser).ok_or(SvgError::Limit)?)
    }

    pub fn parse(bytes: &[u8], limits: Limits, cancelled: &dyn Fn() -> bool) -> SvgResult<Self> {
        limits.validate()?;
        if bytes.len() > limits.input_bytes {
            return Err(SvgError::Limit);
        }
        if bytes.starts_with(&[0x1f, 0x8b]) {
            return Err(SvgError::CompressedUnsupported);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| SvgError::Malformed)?;
        let mut admission = inspect(text, limits, cancelled)?;
        let flags = Arc::new(AtomicU8::new(0));
        let mut fonts = fontdb::Database::new();
        fonts.load_font_data(INTER.to_vec());
        fonts.load_font_data(SANS.to_vec());
        fonts.load_font_data(MONO.to_vec());
        fonts.set_serif_family("Noto Sans");
        fonts.set_sans_serif_family("Noto Sans");
        fonts.set_monospace_family("Noto Sans Mono");
        fonts.set_cursive_family("Noto Sans");
        fonts.set_fantasy_family("Noto Sans");
        let select = FontResolver::default_font_selector();
        let fallback = FontResolver::default_fallback_selector();
        let selection_flags = flags.clone();
        let fallback_flags = flags.clone();
        let options = Options {
            resources_dir: None,
            font_family: "Inter".to_owned(),
            fontdb: Arc::new(fonts),
            image_href_resolver: ImageHrefResolver {
                resolve_data: Box::new(|_, _, _| None),
                resolve_string: Box::new(|_, _| None),
            },
            font_resolver: FontResolver {
                select_font: Box::new(move |font, database| {
                    for family in font.families() {
                        let name = family.to_string();
                        let name = name.trim_matches(['\'', '"']);
                        if !matches!(
                            name,
                            "Inter" | "Noto Sans" | "Noto Sans Mono" | "sans-serif" | "monospace"
                        ) {
                            selection_flags.fetch_or(1, Ordering::Relaxed);
                        }
                    }
                    let result = select(font, database);
                    if result.is_none() {
                        selection_flags.fetch_or(1, Ordering::Relaxed);
                    }
                    result
                }),
                select_fallback: Box::new(move |character, used, database| {
                    if used.is_empty() {
                        fallback_flags.fetch_or(2, Ordering::Relaxed);
                        return None;
                    }
                    let result = fallback(character, used, database);
                    if result.is_none() {
                        fallback_flags.fetch_or(2, Ordering::Relaxed);
                    }
                    result
                }),
            },
            ..Options::default()
        };
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        // Do not call from_str (enables DTD) or from_data (unbounded SVGZ inflate).
        let tree = Tree::from_xmltree(&admission.doc, &options).map_err(|_| SvgError::Malformed)?;
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        let size = tree.size();
        if !size.width().is_finite()
            || !size.height().is_finite()
            || size.width() <= 0.0
            || size.height() <= 0.0
            || f64::from(size.width()) > f64::from(limits.document_side)
            || f64::from(size.height()) > f64::from(limits.document_side)
        {
            return Err(SvgError::DocumentSize);
        }
        let observed = flags.load(Ordering::Relaxed);
        if observed & 1 != 0 {
            admission.notices.insert(FidelityNotice::MissingFont);
        }
        if observed & 2 != 0 {
            admission.notices.insert(FidelityNotice::MissingGlyph);
        }
        Ok(Self {
            tree,
            limits,
            workspace: admission.workspace,
            layers: admission.layers,
            filters: admission.filters,
            notices: admission.notices.into_iter().collect(),
        })
    }

    pub fn info(&self) -> SvgInfo {
        SvgInfo {
            width: f64::from(self.tree.size().width()),
            height: f64::from(self.tree.size().height()),
            notices: self.notices.clone(),
        }
    }

    pub fn render_tile(
        &self,
        request: TileRequest,
        cancelled: &dyn Fn() -> bool,
    ) -> SvgResult<SvgTile> {
        request.validate(self.limits)?;
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        let pixels = u64::from(request.width) * u64::from(request.height);
        // resvg clamps intermediate layer bounds to 5x the tile on each axis.
        // Reserve every declared layer/filter concurrently, conservatively, plus
        // premultiplied output and straight output. The worker is the hard cap.
        let surfaces = self
            .layers
            .checked_add(self.filters.checked_mul(4).ok_or(SvgError::Limit)?)
            .and_then(|v| v.checked_add(2))
            .ok_or(SvgError::Limit)?;
        let required = surfaces
            .checked_mul(pixels)
            .and_then(|v| v.checked_mul(100))
            .and_then(|v| v.checked_add(self.workspace))
            .ok_or(SvgError::Limit)?;
        self.limits.admit(required)?;
        let mut pixmap =
            resvg::tiny_skia::Pixmap::new(request.width, request.height).ok_or(SvgError::Limit)?;
        let transform = resvg::tiny_skia::Transform::from_row(
            request.scale as f32,
            0.0,
            0.0,
            request.scale as f32,
            (-request.x * request.scale) as f32,
            (-request.y * request.scale) as f32,
        );
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        let mut rgba = pixmap.take();
        for pixel in rgba.as_chunks_mut::<4>().0.iter_mut() {
            let alpha = u32::from(pixel[3]);
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 + alpha / 2)
                    .checked_div(alpha)
                    .map_or(0, |value| value.min(255)) as u8;
            }
        }
        Ok(SvgTile {
            width: request.width,
            height: request.height,
            rgba,
        })
    }

    pub(crate) fn limits(&self) -> Limits {
        self.limits
    }
}

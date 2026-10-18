//! Actual engine adapters; a platform entrypoint establishes isolation first.
use pdfium_render::prelude::Pdfium;
use std::{io::Write, path::Path};
use vw_document_protocol::{CopyKind, FaultCode, Job, Operation, Overlay, Request};
use vw_document_win::{DocumentFormat, Glyph, Notice, ResultMetadata};
use vw_pdf::{
    PageRedaction, PdfCancellation, PdfCopyKind, PdfError, PdfLimits, PdfNotice, PdfOverlay,
    PdfPoint, PdfRect, PdfRuntime, PdfTileRequest, RasterRedactionError, RasterRedactionOptions,
    RasterRedactionPlan, RasterSecurityPolicy,
};
use vw_svg::{FidelityNotice, SvgDocument, SvgError};

pub(crate) struct Output {
    pub metadata: ResultMetadata,
    pub bytes: Vec<u8>,
}
const WRAPPER_RESERVE: u64 = 8 * 1024 * 1024;

fn engine_memory(request: &Request) -> Result<u64, FaultCode> {
    // The bounded protocol record/mapped DTOs/result metadata remain retained.
    // Copy output is privately staged in memory before any success metadata;
    // reserve its configured cap, not only the current Vec length.
    let staging = match &request.operation {
        Operation::PdfCopy { .. } => request.limits.output_bytes,
        Operation::PdfRasterRedaction { .. } => {
            request.limits.output_bytes.min(request.limits.input_bytes)
        }
        Operation::SvgCopy { .. } => request.limits.output_bytes.min(4 * 1024 * 1024),
        _ => 0,
    };
    request
        .limits
        .memory_bytes
        .checked_sub(WRAPPER_RESERVE)
        .and_then(|n| n.checked_sub(staging))
        .filter(|n| *n >= 16 * 1024 * 1024)
        .ok_or(FaultCode::Memory)
}

pub(crate) fn execute(job: Job, library: &Path) -> Result<Output, FaultCode> {
    job.request.validate().map_err(|_| FaultCode::Invalid)?;
    job.request
        .validate_source_identity(&job.binding.source_blake3)
        .map_err(|_| FaultCode::Verification)?;
    if job.request.is_copy() {
        job.request
            .require_copy_identity()
            .map_err(|_| FaultCode::Invalid)?;
    }
    if matches!(
        job.request.operation,
        Operation::PdfCopy {
            copy: CopyKind::Original,
            ..
        }
    ) {
        // Exact-original passthrough cannot also add CORE-006 file metadata.
        return Err(FaultCode::Unsupported);
    }
    let memory = engine_memory(&job.request)?;
    let output = if matches!(
        &job.request.operation,
        Operation::SvgInfo | Operation::SvgTile { .. } | Operation::SvgCopy { .. }
    ) {
        svg(&job, memory)?
    } else {
        pdf(job.source, &job.request, memory, library)?
    };
    output
        .metadata
        .validate_for(&job.request)
        .map_err(|_| FaultCode::Verification)?;
    if output.metadata.data_bytes() != output.bytes.len() as u64 {
        return Err(FaultCode::Verification);
    }
    Ok(output)
}
fn svg(job: &Job, memory: u64) -> Result<Output, FaultCode> {
    let limits = vw_svg::Limits {
        input_bytes: usize::try_from(job.request.limits.input_bytes.min(4 * 1024 * 1024))
            .map_err(|_| FaultCode::Limit)?,
        output_bytes: usize::try_from(job.request.limits.output_bytes.min(4 * 1024 * 1024))
            .map_err(|_| FaultCode::Limit)?,
        memory_bytes: memory,
        ..vw_svg::Limits::default()
    };
    let document = SvgDocument::parse(&job.source, limits, &|| false).map_err(svg_error)?;
    let info = document.info();
    let notices = info.notices.into_iter().map(svg_notice).collect();
    match &job.request.operation {
        Operation::SvgInfo => Ok(Output {
            metadata: ResultMetadata::SvgInfo {
                width: info.width,
                height: info.height,
                notices,
            },
            bytes: Vec::new(),
        }),
        Operation::SvgTile { tile } => {
            let result = document
                .render_tile(
                    vw_svg::TileRequest {
                        x: tile.x,
                        y: tile.y,
                        scale: tile.scale,
                        width: tile.width,
                        height: tile.height,
                    },
                    &|| false,
                )
                .map_err(svg_error)?;
            Ok(Output {
                metadata: ResultMetadata::RgbaTile {
                    format: DocumentFormat::Svg,
                    page: None,
                    origin: [tile.x, tile.y],
                    requested_scale: tile.scale,
                    native_scale: f64::from(tile.scale as f32),
                    width: result.width,
                    height: result.height,
                    opaque_paper: false,
                },
                bytes: result.rgba,
            })
        }
        Operation::SvgCopy { overlays } => {
            let identity = job
                .request
                .require_copy_identity()
                .map_err(|_| FaultCode::Invalid)?;
            let metadata =
                ResultMetadata::identity_metadata(identity).map_err(|_| FaultCode::Invalid)?;
            let outlines = overlays
                .iter()
                .map(|overlay| vw_svg::Outline {
                    contours: overlay
                        .contours
                        .iter()
                        .map(|contour| {
                            contour
                                .iter()
                                .map(|point| vw_svg::Point {
                                    x: point[0],
                                    y: point[1],
                                })
                                .collect()
                        })
                        .collect(),
                    transform: overlay.transform,
                    rgba: overlay.rgba,
                })
                .collect::<Vec<_>>();
            let bytes = document
                .export_svg_with_metadata(&outlines, &metadata, &|| false)
                .map_err(svg_error)?;
            Ok(Output {
                metadata: ResultMetadata::SvgCopy {
                    export_identity: identity.clone(),
                    bytes: bytes.len() as u64,
                    notices,
                },
                bytes,
            })
        }
        _ => Err(FaultCode::Invalid),
    }
}
fn pdf(
    source: Vec<u8>,
    request: &Request,
    memory: u64,
    library: &Path,
) -> Result<Output, FaultCode> {
    let limits = PdfLimits {
        input_bytes: request.limits.input_bytes,
        output_bytes: request.limits.output_bytes,
        memory_bytes: memory,
        ..PdfLimits::default()
    };
    let bindings = Pdfium::bind_to_library(library).map_err(|_| FaultCode::Native)?;
    // SAFETY: this adapter is called only by the isolated Android UID gate.
    // The immutable installed library is hash-checked before this call.
    // The one-job service enforces an external process deadline and retires.
    // No other Pdfium instance exists in this one-job process.
    let mut runtime =
        unsafe { PdfRuntime::from_verified_bindings(bindings, limits) }.map_err(pdf_error)?;
    let cancel = PdfCancellation::default();
    let mut document = runtime
        .open(source, request.password.as_deref(), &cancel)
        .map_err(pdf_error)?;
    match &request.operation {
        Operation::PdfInfo => Ok(Output {
            metadata: ResultMetadata::PdfInfo {
                pages: document.info().pages,
                encrypted: document.info().encrypted,
                notices: document
                    .info()
                    .notices
                    .iter()
                    .copied()
                    .map(pdf_notice)
                    .collect(),
            },
            bytes: Vec::new(),
        }),
        Operation::PdfPage { page } => {
            let result = document.page_info(*page, &cancel).map_err(pdf_error)?;
            Ok(Output {
                metadata: ResultMetadata::PdfPage {
                    page: *page,
                    width: result.geometry.width(),
                    height: result.geometry.height(),
                    crop: result.geometry.crop_pdf(),
                    quarter_turns: result.geometry.quarter_turns(),
                    objects: result.objects,
                    annotations: result.annotations,
                },
                bytes: Vec::new(),
            })
        }
        Operation::PdfTile {
            page,
            tile,
            annotations,
        } => {
            let result = document
                .render_tile(
                    *page,
                    PdfTileRequest {
                        x: tile.x,
                        y: tile.y,
                        pixels_per_point: tile.scale,
                        width: tile.width,
                        height: tile.height,
                        include_annotations: *annotations,
                    },
                    &cancel,
                )
                .map_err(pdf_error)?;
            Ok(Output {
                metadata: ResultMetadata::RgbaTile {
                    format: DocumentFormat::Pdf,
                    page: Some(*page),
                    origin: [tile.x, tile.y],
                    requested_scale: tile.scale,
                    native_scale: f64::from(tile.scale as f32),
                    width: result.width,
                    height: result.height,
                    opaque_paper: true,
                },
                bytes: result.rgba,
            })
        }
        Operation::PdfText { page, start, count } => {
            let result = document
                .text_chunk(*page, *start, *count, &cancel)
                .map_err(pdf_error)?;
            Ok(Output {
                metadata: ResultMetadata::PdfText {
                    page: *page,
                    total_characters: result.total_characters,
                    start: result.start,
                    next: result.next,
                    glyphs: result.glyphs.into_iter().map(glyph).collect(),
                },
                bytes: Vec::new(),
            })
        }
        Operation::PdfSelection { page, rectangle } => {
            let result = document
                .select_text(*page, rect(*rectangle), &cancel)
                .map_err(pdf_error)?;
            Ok(Output {
                metadata: ResultMetadata::PdfSelection {
                    page: *page,
                    glyphs: result.into_iter().map(glyph).collect(),
                },
                bytes: Vec::new(),
            })
        }
        Operation::PdfCopy { copy, overlays } => {
            let identity = request
                .require_copy_identity()
                .map_err(|_| FaultCode::Invalid)?;
            let metadata =
                ResultMetadata::identity_metadata(identity).map_err(|_| FaultCode::Invalid)?;
            let kind = match copy {
                CopyKind::Original => PdfCopyKind::Original,
                CopyKind::Annotated => PdfCopyKind::Annotated,
                CopyKind::Flattened => PdfCopyKind::Flattened,
            };
            let overlays: Vec<_> = overlays.iter().map(pdf_overlay).collect();
            let mut stage = BoundedBytes::new(request.limits.output_bytes)?;
            let result = document
                .export(kind, &overlays, &mut stage, &cancel)
                .map_err(pdf_error)?;
            if result.redacted || result.bytes != stage.bytes.len() as u64 {
                return Err(FaultCode::Verification);
            }
            drop(document);
            vw_pdf::embed_pdf_copy_metadata(&mut stage.bytes, &metadata, limits, &cancel)
                .map_err(pdf_error)?;
            let generated = runtime
                .open(stage.bytes, None, &cancel)
                .map_err(pdf_error)?;
            generated
                .verify_export_metadata(&metadata, &cancel)
                .map_err(|_| FaultCode::Verification)?;
            let mut published = BoundedBytes::new(request.limits.output_bytes)?;
            // Exact passthrough of this freshly verified generated document.
            let mut generated = generated;
            generated
                .export(PdfCopyKind::Original, &[], &mut published, &cancel)
                .map_err(pdf_error)?;
            vw_pdf::verify_pdf_copy_metadata(
                &published.bytes,
                &metadata,
                request.limits.output_bytes,
                &cancel,
            )
            .map_err(|_| FaultCode::Verification)?;
            Ok(Output {
                metadata: ResultMetadata::PdfCopy {
                    export_identity: identity.clone(),
                    copy: *copy,
                    pages: result.pages,
                    bytes: published.bytes.len() as u64,
                    redacted: false,
                },
                bytes: published.bytes,
            })
        }
        Operation::PdfRasterRedaction {
            scale,
            explicitly_remove_encryption,
            regions,
        } => {
            let identity = request
                .require_copy_identity()
                .map_err(|_| FaultCode::Invalid)?;
            let metadata =
                ResultMetadata::identity_metadata(identity).map_err(|_| FaultCode::Invalid)?;
            let copy_limits = PdfLimits {
                output_bytes: limits.output_bytes.min(limits.input_bytes),
                ..limits
            };
            let plan = RasterRedactionPlan::prepare(
                &mut document,
                regions
                    .iter()
                    .map(|region| PageRedaction {
                        page: region.page,
                        region: rect(region.rectangle),
                    })
                    .collect(),
                RasterRedactionOptions {
                    pixels_per_point: *scale,
                    security: if *explicitly_remove_encryption {
                        RasterSecurityPolicy::ExplicitlyRemoveEncryption
                    } else {
                        RasterSecurityPolicy::RefuseEncrypted
                    },
                },
                &cancel,
            )
            .map_err(redaction_error)?;
            let copy_limits = PdfLimits {
                output_bytes: copy_limits
                    .output_bytes
                    .min(plan.work().maximum_encoded_bytes),
                ..copy_limits
            };
            let mut stage =
                BoundedBytes::new(request.limits.output_bytes.min(request.limits.input_bytes))?;
            let intermediate = plan
                .write_unverified(&mut document, &mut stage, &cancel, &mut |_| {})
                .map_err(redaction_error)?;
            let intermediate = intermediate
                .embed_metadata(&mut stage.bytes, &metadata, copy_limits, &cancel)
                .map_err(redaction_error)?;
            drop(document);
            // Reopen the exact completed bytes, without copying a second Vec.
            let mut generated = runtime
                .open(stage.bytes, None, &cancel)
                .map_err(pdf_error)?;
            generated
                .verify_export_metadata(&metadata, &cancel)
                .map_err(|_| FaultCode::Verification)?;
            let verified = plan
                .verify(&mut generated, intermediate, &cancel)
                .map_err(redaction_error)?;
            // Original export of the verified generated document emits exactly
            // its retained fresh PDF; reserve for this second stage explicitly.
            let mut published =
                BoundedBytes::new(request.limits.output_bytes.min(request.limits.input_bytes))?;
            generated
                .export(PdfCopyKind::Original, &[], &mut published, &cancel)
                .map_err(pdf_error)?;
            if published.bytes.len() as u64 != verified.bytes() {
                return Err(FaultCode::Verification);
            }
            vw_pdf::verify_pdf_copy_metadata(
                &published.bytes,
                &metadata,
                copy_limits.output_bytes,
                &cancel,
            )
            .map_err(|_| FaultCode::Verification)?;
            Ok(Output {
                metadata: ResultMetadata::RasterRedaction {
                    export_identity: identity.clone(),
                    pages: verified.pages(),
                    regions: verified.regions(),
                    bytes: verified.bytes(),
                    requested_scale: verified.requested_pixels_per_point(),
                    effective_scale: verified.pixels_per_point(),
                    plan_blake3: verified.plan_blake3().to_owned(),
                },
                bytes: published.bytes,
            })
        }
        _ => Err(FaultCode::Invalid),
    }
}
fn pdf_overlay(value: &Overlay) -> PdfOverlay {
    PdfOverlay {
        page: value.page,
        rgba: value.rgba,
        transform: value.transform,
        contours: value
            .contours
            .iter()
            .map(|path| {
                path.iter()
                    .map(|point| PdfPoint {
                        x: point[0],
                        y: point[1],
                    })
                    .collect()
            })
            .collect(),
    }
}
fn rect(value: [f64; 4]) -> PdfRect {
    PdfRect {
        left: value[0],
        top: value[1],
        right: value[2],
        bottom: value[3],
    }
}
fn glyph(value: vw_pdf::PdfGlyph) -> Glyph {
    Glyph {
        index: value.index,
        unicode: value.unicode,
        generated: value.generated,
        bounds: value.bounds.map(|r| [r.left, r.top, r.right, r.bottom]),
    }
}
fn pdf_notice(value: PdfNotice) -> Notice {
    match value {
        PdfNotice::SystemFontsDisabled => Notice::SystemFontsDisabled,
        PdfNotice::NonInteractiveForms => Notice::NonInteractiveForms,
        PdfNotice::OriginalDocumentFeaturesRetained => Notice::OriginalDocumentFeaturesRetained,
    }
}
fn svg_notice(value: FidelityNotice) -> Notice {
    match value {
        FidelityNotice::ScriptOrEventIgnored => Notice::ScriptOrEventIgnored,
        FidelityNotice::ForeignObjectIgnored => Notice::ForeignObjectIgnored,
        FidelityNotice::AnimationIgnored => Notice::AnimationIgnored,
        FidelityNotice::ResourceDenied => Notice::ResourceDenied,
        FidelityNotice::StylesheetLimited => Notice::StylesheetLimited,
        FidelityNotice::UnsupportedFilter => Notice::UnsupportedFilter,
        FidelityNotice::UnsupportedElement => Notice::UnsupportedElement,
        FidelityNotice::MissingFont => Notice::MissingFont,
        FidelityNotice::MissingGlyph => Notice::MissingGlyph,
    }
}
fn pdf_error(value: PdfError) -> FaultCode {
    match value {
        PdfError::Limit => FaultCode::Limit,
        PdfError::Memory { .. } | PdfError::Allocation => FaultCode::Memory,
        PdfError::Cancelled => FaultCode::Cancelled,
        PdfError::Password => FaultCode::Password,
        PdfError::UnsupportedSecurity
        | PdfError::EncryptedExport
        | PdfError::UnsupportedForms
        | PdfError::UnsupportedGeometry => FaultCode::Unsupported,
        PdfError::Malformed => FaultCode::Malformed,
        PdfError::Invalid => FaultCode::Invalid,
        PdfError::Io(_) | PdfError::WriterPanicked => FaultCode::Output,
        PdfError::Busy | PdfError::Native => FaultCode::Native,
    }
}
fn svg_error(value: SvgError) -> FaultCode {
    match value {
        SvgError::Limit | SvgError::DocumentSize => FaultCode::Limit,
        SvgError::Memory { .. } => FaultCode::Memory,
        SvgError::Cancelled => FaultCode::Cancelled,
        SvgError::Malformed => FaultCode::Malformed,
        SvgError::DtdDenied
        | SvgError::CompressedUnsupported
        | SvgError::UnsupportedCss
        | SvgError::ReferenceCycle => FaultCode::Unsupported,
        SvgError::InvalidLimits | SvgError::InvalidTile | SvgError::InvalidOverlay => {
            FaultCode::Invalid
        }
    }
}
fn redaction_error(value: RasterRedactionError) -> FaultCode {
    match value {
        RasterRedactionError::Pdf(error) => pdf_error(error),
        RasterRedactionError::Invalid => FaultCode::Invalid,
        RasterRedactionError::Limit => FaultCode::Limit,
        RasterRedactionError::SourceMismatch | RasterRedactionError::Verification => {
            FaultCode::Verification
        }
    }
}

struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl BoundedBytes {
    fn new(limit: u64) -> Result<Self, FaultCode> {
        Ok(Self {
            bytes: Vec::new(),
            limit: usize::try_from(limit).map_err(|_| FaultCode::Limit)?,
        })
    }
}
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .ok_or(std::io::ErrorKind::FileTooLarge)?;
        if next > self.limit {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        if next > self.bytes.capacity() {
            let capacity = next
                .max(self.bytes.capacity().saturating_mul(2).max(4096))
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| std::io::ErrorKind::OutOfMemory)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use vw_document_protocol::JobLimits;
    fn request(operation: Operation, memory_bytes: u64, output_bytes: u64) -> Request {
        Request {
            operation,
            password: None,
            export_identity: None,
            limits: JobLimits {
                input_bytes: 64 * 1024 * 1024,
                memory_bytes,
                output_bytes,
            },
        }
    }
    #[test]
    fn svg_copy_embeds_exact_identity_before_typed_result_adoption() {
        let source=b"<svg xmlns='http://www.w3.org/2000/svg' width='8' height='8'><rect width='8' height='8' fill='red'/></svg>".to_vec();
        let hash = blake3::hash(&source);
        let identity = vw_document_protocol::CopyIdentity {
            schema_version: 1,
            host_seq: u64::MAX,
            state_hash: "b".repeat(64),
            source_asset: hash.to_hex().to_string(),
            capture_session: None,
            frame_id: None,
        };
        let mut request = request(
            Operation::SvgCopy { overlays: vec![] },
            512 * 1024 * 1024,
            8 * 1024 * 1024,
        );
        request.export_identity = Some(identity.clone());
        let output = execute(
            Job {
                binding: vw_document_protocol::JobBinding {
                    nonce: [7; 16],
                    source_blake3: *hash.as_bytes(),
                },
                request: request.clone(),
                source,
            },
            Path::new(""),
        )
        .unwrap();
        assert_eq!(output.metadata.export_identity(), Some(&identity));
        let metadata = ResultMetadata::identity_metadata(&identity).unwrap();
        vw_svg::verify_svg_copy_metadata(
            &output.bytes,
            &metadata,
            vw_svg::Limits::default(),
            &|| false,
        )
        .unwrap();
        let result = super::super::DocumentJobResult {
            source_blake3: identity.source_asset.clone(),
            metadata_json: serde_json::to_string(&output.metadata).unwrap(),
            output_blake3: blake3::hash(&output.bytes).to_hex().to_string(),
            bytes: output.bytes,
        };
        super::super::validate_document_job_result(
            serde_json::to_string(&request).unwrap(),
            identity.source_asset.clone(),
            result,
        )
        .unwrap();
    }
    #[test]
    fn copy_stage_capacity_is_reserved_before_native_binding_or_decode() {
        let value = request(
            Operation::PdfCopy {
                copy: CopyKind::Original,
                overlays: vec![],
            },
            64 * 1024 * 1024,
            64 * 1024 * 1024,
        );
        assert_eq!(engine_memory(&value), Err(FaultCode::Memory));
    }
    #[test]
    fn raster_reopen_source_and_second_copy_stage_are_both_accounted() {
        let value = request(
            Operation::PdfRasterRedaction {
                scale: 1.,
                explicitly_remove_encryption: false,
                regions: vec![vw_document_protocol::PageRegion {
                    page: 0,
                    rectangle: [0., 0., 1., 1.],
                }],
            },
            192 * 1024 * 1024,
            128 * 1024 * 1024,
        );
        assert_eq!(engine_memory(&value).unwrap(), 120 * 1024 * 1024);
        // Generated input is then charged inside this remaining120 MiB by
        // PdfLimits; the separate64 MiB stage is not counted as free memory.
    }
    #[test]
    fn bounded_stage_refuses_before_appending_and_preserves_accepted_bytes() {
        let mut stage = BoundedBytes::new(4).unwrap();
        stage.write_all(b"ab").unwrap();
        assert!(stage.write_all(b"cde").is_err());
        assert_eq!(stage.bytes, b"ab");
        assert!(stage.bytes.capacity() <= 4);
    }
    #[test]
    fn typed_faults_do_not_relay_native_error_text_or_paths() {
        assert_eq!(pdf_error(PdfError::Password), FaultCode::Password);
        assert_eq!(
            pdf_error(PdfError::Io(std::io::ErrorKind::PermissionDenied)),
            FaultCode::Output
        );
        assert_eq!(svg_error(SvgError::UnsupportedCss), FaultCode::Unsupported);
    }
}

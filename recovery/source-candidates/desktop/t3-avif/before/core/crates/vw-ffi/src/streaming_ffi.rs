//! File-backed raster operations. The native worker owns every scratch handle;
//! only complete no-clobber outputs leave its private staging area.
use crate::{
    project::{ProjectState, SessionPermit},
    *,
};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_proto::v1 as pb;
use vw_raster::{
    AssetResolver, BorrowedSource, DecodedImage, JpegSpool, PngSpool, RasterSource, SourceInfo,
    SpoolLimits,
};

const INPUT_LIMIT: u64 = 64 * 1024 * 1024;
const MEMORY_LIMIT: u64 = 256 * 1024 * 1024;
const OUTPUT_LIMIT: u64 = 512 * 1024 * 1024;
const SCRATCH_LIMIT: u64 = 400_000_000;

/// Stable preflight categories: callers can offer PNG, a smaller region, an
/// explicit matte/conversion, or a larger budget without parsing error text.
#[derive(Clone, Debug, thiserror::Error, uniffi::Error)]
pub enum TransferError {
    #[error("image dimensions exceed the {limit}px format limit")]
    Dimensions { width: u32, height: u32, limit: u32 },
    #[error("image exceeds the import pixel-count limit")]
    PixelLimit {
        width: u32,
        height: u32,
        max_pixels: u64,
    },
    #[error("memory estimate exceeds the supplied budget")]
    Memory { estimated: u64, budget: u64 },
    #[error("image metadata or color profile is invalid or unsupported")]
    Metadata,
    #[error("JPEG needs an explicit matte for alpha")]
    Alpha,
    #[error("depth reduction needs explicit permission")]
    Depth,
    #[error("encoded output exceeds its limit")]
    EncodedLimit,
    #[error("private scratch exceeds its limit")]
    ScratchLimit,
    #[error("unsupported transfer format")]
    Unsupported,
    #[error("invalid transfer request")]
    Invalid,
    #[error("transfer storage failure")]
    Storage,
    #[error("transfer cancelled")]
    Cancelled,
    #[error("project closed")]
    Closed,
    #[error("transfer capacity exhausted")]
    Backpressure,
}
pub type TransferResult<T> = std::result::Result<T, TransferError>;
impl From<CoreError> for TransferError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Cancelled => Self::Cancelled,
            CoreError::Closed => Self::Closed,
            CoreError::Storage => Self::Storage,
            CoreError::Backpressure | CoreError::Worker => Self::Backpressure,
            CoreError::Unsupported => Self::Unsupported,
            CoreError::Raster => Self::Metadata,
            _ => Self::Invalid,
        }
    }
}
impl From<std::io::Error> for TransferError {
    fn from(_: std::io::Error) -> Self {
        Self::Storage
    }
}
impl From<vw_store::StoreError> for TransferError {
    fn from(_: vw_store::StoreError) -> Self {
        Self::Storage
    }
}
impl From<vw_model::ModelError> for TransferError {
    fn from(_: vw_model::ModelError) -> Self {
        Self::Invalid
    }
}
impl From<TransferError> for CoreError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::Cancelled => Self::Cancelled,
            TransferError::Closed => Self::Closed,
            TransferError::Storage => Self::Storage,
            TransferError::Memory { .. }
            | TransferError::EncodedLimit
            | TransferError::ScratchLimit
            | TransferError::Backpressure => Self::Backpressure,
            TransferError::Unsupported => Self::Unsupported,
            TransferError::Invalid => Self::Invalid,
            _ => Self::Raster,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FilePreflight {
    pub width: u32,
    pub height: u32,
    pub source_bit_depth: u8,
    pub output_bit_depth: u8,
    pub estimated_peak_bytes: u64,
    pub buffered: bool,
    pub requires_render_validation: bool,
    pub revision: ProjectInfo,
}
#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum FileAssetKind {
    Image,
    Mp4,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AttachFileOptions {
    pub transaction_id: String,
    pub device_id: String,
    pub lamport: u64,
    pub now_ms: i64,
    pub source_path: String,
    pub work_directory: String,
    pub kind: FileAssetKind,
    pub memory_budget_bytes: u64,
    pub max_encoded_bytes: u64,
    pub max_scratch_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct AttachedAsset {
    pub asset_id: String,
    pub byte_size: u64,
    pub format: String,
    pub revision: ProjectInfo,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct CreateFileImageProject {
    pub path: String,
    pub project_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub device_id: String,
    pub title: String,
    pub source_path: String,
    pub work_directory: String,
    pub now_ms: i64,
    pub memory_budget_bytes: u64,
    pub max_encoded_bytes: u64,
    pub max_scratch_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FileExportOptions {
    pub export: ExportOptions,
    pub output_path: String,
    pub work_directory: String,
    pub max_encoded_bytes: u64,
    pub max_scratch_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct FileExportResult {
    pub encoded_bytes: u64,
    pub blake3: String,
    pub metadata_json: String,
    pub revision: ProjectInfo,
    /// Compositor/encoder plan reservation; the prior source decode is checked
    /// separately against the same budget. This is not process RSS or a measured
    /// peak across the entire transfer and does not include the application UI.
    pub estimated_peak_bytes: u64,
}

#[uniffi::export]
pub async fn create_file_image_project(
    options: CreateFileImageProject,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<ProjectSession>> {
    create_file_project_inner(options, cancellation, None).await
}
pub(crate) async fn create_file_project_inner(
    options: CreateFileImageProject,
    cancellation: Arc<Cancellation>,
    capture: Option<crate::capture_import::ValidatedCapture>,
) -> Result<Arc<ProjectSession>> {
    validate_path(&options.path)?;
    validate_path(&options.source_path)?;
    limits(
        options.memory_budget_bytes,
        options.max_encoded_bytes,
        INPUT_LIMIT,
        options.max_scratch_bytes,
    )?;
    if options.title.len() > 4096 || options.now_ms < 0 {
        return Err(CoreError::Invalid);
    }
    crate::worker::startup(move || {
        cancellation.check()?;
        let permit = SessionPermit::acquire()?;
        let project_id = Id::try_from(options.project_id)?;
        let document_id = Id::try_from(options.document_id)?;
        let layer_id = Id::try_from(options.layer_id)?;
        let device = DeviceId::try_from(options.device_id)?;
        let work = work_directory(&options.work_directory)?;
        let bytes = read_bounded(
            Path::new(&options.source_path),
            options.max_encoded_bytes.min(options.memory_budget_bytes),
            &cancellation,
        )?;
        let mut scratch = tempfile::Builder::new()
            .prefix("vw-raster-source-")
            .tempfile_in(&work)?;
        let (info, alpha) = with_source(
            &bytes,
            scratch.as_file_mut(),
            options.memory_budget_bytes,
            options.max_scratch_bytes,
            &cancellation,
            |source| {
                admit_scan(source, options.memory_budget_bytes)?;
                let info = source.info().clone();
                let mut alpha = false;
                for y in (0..info.height).step_by(32) {
                    cancellation.check()?;
                    let region = vw_raster::Region {
                        x: 0,
                        y,
                        width: info.width,
                        height: 32.min(info.height - y),
                    };
                    if source
                        .read_region(region, options.memory_budget_bytes, &|| {
                            cancellation.is_cancelled()
                        })
                        .map_err(raster_error)?
                        .pixels
                        .has_alpha()
                    {
                        alpha = true;
                        break;
                    }
                }
                Ok((info, alpha))
            },
        )?;
        scratch.close()?;
        if let Some(capture) = &capture {
            capture.verify_source(&info, &bytes)?;
        }
        let mut project = Project::new(project_id, options.title.clone(), device.clone());
        let (width, height) = if info.orientation_applied >= 5 {
            (info.height, info.width)
        } else {
            (info.width, info.height)
        };
        let format = if vw_codec_os::is_heic(&bytes) {
            "heic"
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "png"
        } else if bytes.starts_with(&[0xff, 0xd8]) {
            "jpeg"
        } else {
            "webp"
        };
        project.assets.insert(
            info.source_asset.clone(),
            pb::AddAsset {
                asset_id: info.source_asset.to_string(),
                format: format.into(),
                width,
                height,
                orientation: u32::from(info.orientation_applied),
                bit_depth: u32::from(info.bit_depth),
                has_alpha: alpha,
                color_space: if info.icc.is_some() {
                    "ICC"
                } else {
                    "untagged"
                }
                .into(),
                icc_profile: info.icc.unwrap_or_default(),
                byte_size: bytes.len() as u64,
                source: if capture.is_some() {
                    "capture"
                } else {
                    "import"
                }
                .into(),
                captured_at_ms: capture
                    .as_ref()
                    .map_or(0, |value| value.info().captured_at_ms),
                metadata_json: "{}".into(),
            },
        );
        project.documents.insert(
            document_id.clone(),
            Document {
                definition: pb::CreateDocument {
                    document_id: Some(document_id.to_proto()),
                    kind: if capture.is_some() {
                        pb::DocumentKind::Capture
                    } else {
                        pb::DocumentKind::Image
                    } as i32,
                    capture: capture.as_ref().map(|value| value.info()),
                    schema_version: 1,
                    title: options.title,
                    primary_asset_id: info.source_asset.to_string(),
                },
                pages: vec![],
                created_at_ms: options.now_ms,
            },
        );
        project.layers.insert(
            layer_id.clone(),
            Layer {
                definition: pb::CreateLayer {
                    layer_id: Some(layer_id.to_proto()),
                    document_id: Some(document_id.to_proto()),
                    page_index: -1,
                    name: "Annotations".into(),
                    kind: "annotation".into(),
                    order_key: "V".into(),
                },
                visible: true,
                locked: false,
                opacity: 1.0,
                blend: "normal".into(),
            },
        );
        project.validate()?;
        cancellation.check()?;
        // create_complete owns staged DB/blob cleanup and checks cancellation
        // immediately before no-replace publication. Published originals survive.
        let (store, blobs) = crate::creation::create_complete(
            Path::new(&options.path),
            crate::creation::InitialProject {
                project,
                device,
                time: options.now_ms,
            },
            &bytes,
            &info.source_asset,
            &cancellation,
            |_| Ok(()),
        )?;
        ProjectSession::from_store(store, blobs, permit)
    })
    .await
}

#[uniffi::export]
impl ProjectSession {
    pub async fn export_image_file(
        &self,
        options: FileExportOptions,
        cancellation: Arc<Cancellation>,
    ) -> Result<FileExportResult> {
        self.export_transfer_file(options, cancellation)
            .await
            .map_err(CoreError::from)
    }
    pub async fn preflight_image_file(
        &self,
        options: FileExportOptions,
        cancellation: Arc<Cancellation>,
    ) -> TransferResult<FilePreflight> {
        self.check_open()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    cancellation.check()?;
                    if closed.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(TransferError::Closed);
                    }
                    preflight_verified(state, &options, &cancellation)
                })())
            })
            .await?
    }
    pub async fn export_transfer_file(
        &self,
        options: FileExportOptions,
        cancellation: Arc<Cancellation>,
    ) -> TransferResult<FileExportResult> {
        self.check_open()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                cancellation.check()?;
                if closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(CoreError::Closed);
                }
                let revision = state.info()?;
                state
                    .events
                    .publish(ChangeKind::ExportStarted, revision.clone());
                let result = preflight_file(state, &options)
                    .and_then(|_| export_file(state, options, &cancellation));
                state.events.publish(
                    if result.is_ok() {
                        ChangeKind::ExportReady
                    } else {
                        ChangeKind::ExportFailed
                    },
                    revision,
                );
                Ok(result)
            })
            .await?
    }
    pub async fn attach_asset_file(
        &self,
        options: AttachFileOptions,
        cancellation: Arc<Cancellation>,
    ) -> TransferResult<AttachedAsset> {
        self.check_open()?;
        let closed = self.closed.clone();
        self.worker
            .call(move |state| {
                if closed.load(std::sync::atomic::Ordering::Acquire) {
                    return Err(CoreError::Closed);
                }
                Ok(attach_file(state, options, &cancellation))
            })
            .await?
    }
}
fn profile_reserve(present: bool) -> u64 {
    if present { 72 * 1024 * 1024 } else { 0 }
}
fn admit_scan(source: &dyn RasterSource, budget: u64) -> TransferResult<()> {
    let info = source.info();
    let estimated = source
        .resident_bytes()
        .checked_add(source.read_workspace_bytes())
        .and_then(|v| v.checked_add(u64::from(info.width) * 32 * 8))
        .and_then(|v| v.checked_add(profile_reserve(info.icc.is_some())))
        .ok_or(TransferError::Backpressure)?;
    memory(estimated, budget)?;
    Ok(())
}
fn preflight_verified(
    state: &ProjectState,
    options: &FileExportOptions,
    cancel: &Cancellation,
) -> TransferResult<FilePreflight> {
    let mut result = preflight_file(state, options)?;
    let project = state.project()?;
    let document = Id::try_from(options.export.document_id.clone())?;
    let definition = &project
        .documents
        .get(&document)
        .ok_or(TransferError::Invalid)?
        .definition;
    let asset = AssetId::try_from(definition.primary_asset_id.clone())?;
    let request = request(&options.export, definition, state.view_revision()?)?;
    let work = work_directory(&options.work_directory)?;
    destination(&options.output_path, &work)?;
    let bytes = read_bounded(
        &state.blobs.path(&asset)?,
        INPUT_LIMIT.min(request.memory_budget_bytes),
        cancel,
    )?;
    if AssetId::hash(&bytes) != asset {
        return Err(TransferError::Metadata);
    }
    let mut scratch = tempfile::Builder::new()
        .prefix("vw-raster-source-")
        .tempfile_in(work)?;
    with_source(
        &bytes,
        scratch.as_file_mut(),
        request.memory_budget_bytes,
        options.max_scratch_bytes,
        cancel,
        |source| {
            validate_source_binding(source.info(), project, &asset)?;
            if options.export.convert_to_srgb
                && source.info().icc.is_none()
                && !options.export.assume_untagged_srgb
            {
                return Err(TransferError::Metadata);
            }
            if result.buffered {
                let (estimate, _, _) = buffered_admission(source, &request)?;
                result.estimated_peak_bytes = result.estimated_peak_bytes.max(estimate);
            }
            if matches!(
                request.format,
                vw_raster::ExportFormat::Png8 | vw_raster::ExportFormat::Png16
            ) {
                let plan = vw_raster::plan_png(
                    &SourceView(source),
                    &request,
                    vw_raster::StreamLimits {
                        max_encoded_bytes: options.max_encoded_bytes,
                        max_strip_rows: 128,
                        max_pixels: 50_000_000,
                    },
                )
                .map_err(transfer_raster_error)?;
                result.estimated_peak_bytes =
                    result.estimated_peak_bytes.max(plan.estimated_peak_bytes);
            } else if matches!(request.format, vw_raster::ExportFormat::Jpeg { .. })
                && matches!(request.alpha, vw_raster::AlphaPolicy::Preserve)
            {
                admit_scan(source, request.memory_budget_bytes)?;
                let region = request.region.unwrap_or(vw_raster::Region {
                    x: 0,
                    y: 0,
                    width: source.info().width,
                    height: source.info().height,
                });
                for y in (region.y..region.y + region.height).step_by(32) {
                    cancel.check()?;
                    let part = vw_raster::Region {
                        x: region.x,
                        y,
                        width: region.width,
                        height: 32.min(region.y + region.height - y),
                    };
                    if source
                        .read_region(part, request.memory_budget_bytes, &|| cancel.is_cancelled())
                        .map_err(transfer_raster_error)?
                        .pixels
                        .has_alpha()
                    {
                        return Err(TransferError::Alpha);
                    }
                }
            }
            Ok(())
        },
    )?;
    scratch.close()?;
    Ok(result)
}
fn memory(estimated: u64, budget: u64) -> TransferResult<u64> {
    if estimated > budget {
        Err(TransferError::Memory { estimated, budget })
    } else {
        Ok(estimated)
    }
}
fn preflight_file(
    state: &ProjectState,
    options: &FileExportOptions,
) -> TransferResult<FilePreflight> {
    limits(
        options.export.memory_budget_bytes,
        options.max_encoded_bytes,
        OUTPUT_LIMIT,
        options.max_scratch_bytes,
    )?;
    let project = state.project()?;
    let document = Id::try_from(options.export.document_id.clone())?;
    let definition = &project
        .documents
        .get(&document)
        .ok_or(TransferError::Invalid)?
        .definition;
    let asset = project
        .assets
        .get(&AssetId::try_from(definition.primary_asset_id.clone())?)
        .ok_or(TransferError::Metadata)?;
    let (width, height) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    if u64::from(width) * u64::from(height) > 50_000_000 {
        return Err(TransferError::PixelLimit {
            width,
            height,
            max_pixels: 50_000_000,
        });
    }
    if asset.byte_size > INPUT_LIMIT {
        return Err(TransferError::EncodedLimit);
    }
    let mut request = request(&options.export, definition, state.view_revision()?)?;
    let region = request.region.unwrap_or(vw_raster::Region {
        x: 0,
        y: 0,
        width,
        height,
    });
    let buffered = !matches!(
        request.format,
        vw_raster::ExportFormat::Png8 | vw_raster::ExportFormat::Png16
    );
    let budget = request.memory_budget_bytes;
    request.memory_budget_bytes = u64::MAX;
    let codec = vw_raster::preflight(
        width,
        height,
        asset.bit_depth as u8,
        asset.has_alpha && request.region.is_none(),
        &request,
    )
    .map_err(|error| match error {
        vw_raster::RasterError::Dimensions { limit, .. } => TransferError::Dimensions {
            width: region.width,
            height: region.height,
            limit,
        },
        other => transfer_raster_error(other),
    })?;
    let full = u64::from(width) * u64::from(height) * 8;
    let profile = profile_reserve(!asset.icc_profile.is_empty());
    let estimate = if buffered {
        asset.byte_size + full + profile + codec
    } else {
        asset.byte_size + profile + 32 * 1024 * 1024 + u64::from(region.width) * 64
    };
    let source_decode = if matches!(asset.format.as_str(), "heic" | "heif") {
        u64::from(width.div_ceil(64) * 64)
            .checked_mul(u64::from(height.div_ceil(64) * 64))
            .and_then(|n| n.checked_mul(64))
            .and_then(|n| n.checked_add(asset.byte_size.checked_mul(5)?))
            .and_then(|n| n.checked_add(32 * 1024 * 1024))
            .ok_or(TransferError::Backpressure)?
    } else {
        0
    };
    let estimate = estimate.max(source_decode);
    memory(estimate, budget)?;
    Ok(FilePreflight {
        width: region.width,
        height: region.height,
        source_bit_depth: asset.bit_depth as u8,
        output_bit_depth: if matches!(request.format, vw_raster::ExportFormat::Png16) {
            16
        } else {
            8
        },
        estimated_peak_bytes: estimate,
        buffered,
        requires_render_validation: options.export.marked,
        revision: state.info()?,
    })
}
fn validate_source_binding(
    info: &SourceInfo,
    project: &Project,
    id: &AssetId,
) -> TransferResult<()> {
    let asset = project.assets.get(id).ok_or(TransferError::Metadata)?;
    let (w, h) = if asset.orientation >= 5 {
        (asset.height, asset.width)
    } else {
        (asset.width, asset.height)
    };
    if info.source_asset != *id
        || info.width != w
        || info.height != h
        || u32::from(info.bit_depth) != asset.bit_depth
        || u32::from(info.orientation_applied) != asset.orientation
        || info.icc.as_deref().unwrap_or_default() != asset.icc_profile
    {
        return Err(TransferError::Metadata);
    }
    Ok(())
}
fn buffered_admission(
    source: &dyn RasterSource,
    request: &vw_raster::ExportRequest,
) -> TransferResult<(u64, u64, vw_raster::ExportRequest)> {
    let info = source.info();
    let retained = source
        .resident_bytes()
        .checked_add(source.read_workspace_bytes())
        .and_then(|v| v.checked_add(profile_reserve(info.icc.is_some())))
        .ok_or(TransferError::Backpressure)?;
    let pixels = u64::from(info.width) * u64::from(info.height) * u64::from(info.bit_depth) / 2;
    memory(
        retained
            .checked_add(pixels)
            .ok_or(TransferError::Backpressure)?,
        request.memory_budget_bytes,
    )?;
    let available = request.memory_budget_bytes - retained;
    let mut admit = request.clone();
    admit.memory_budget_bytes = available.checked_sub(pixels).ok_or(TransferError::Memory {
        estimated: retained + pixels,
        budget: request.memory_budget_bytes,
    })?;
    let codec = vw_raster::preflight(info.width, info.height, info.bit_depth, false, &admit)
        .map_err(transfer_raster_error)?;
    Ok((retained + pixels + codec, available, admit))
}
fn export_buffered(
    source: &mut dyn RasterSource,
    state: &ProjectState,
    document: &Id,
    options: &FileExportOptions,
    request: &vw_raster::ExportRequest,
    cancel: &Cancellation,
    sink: &mut impl Write,
) -> TransferResult<vw_raster::StreamReport> {
    let info = source.info();
    let region = vw_raster::Region {
        x: 0,
        y: 0,
        width: info.width,
        height: info.height,
    };
    let (_, available, admit) = buffered_admission(source, request)?;
    cancel.check()?;
    let mut image = source
        .read_region(region, request.memory_budget_bytes, &|| {
            cancel.is_cancelled()
        })
        .map_err(transfer_raster_error)?;
    if options.export.marked {
        let assets = ResultAssets {
            blobs: &state.blobs,
            budget: available,
            cancel,
        };
        image = vw_raster::render_document(
            &image,
            state.project()?,
            document,
            &assets,
            vw_raster::RenderOptions {
                memory_budget_bytes: available,
                include_guides: false,
                assume_untagged_srgb: options.export.assume_untagged_srgb,
            },
        )
        .map_err(transfer_raster_error)?;
    }
    cancel.check()?;
    let encoded = vw_raster::export(&image, &admit).map_err(transfer_raster_error)?;
    if encoded.bytes.len() as u64 > options.max_encoded_bytes {
        return Err(TransferError::EncodedLimit);
    }
    for chunk in encoded.bytes.chunks(64 * 1024) {
        cancel.check()?;
        sink.write_all(chunk)?;
    }
    let output_region = request.region.unwrap_or(region);
    Ok(vw_raster::StreamReport {
        metadata: encoded.metadata,
        plan: vw_raster::StreamPlan {
            region: output_region,
            strip_rows: output_region.height,
            estimated_peak_bytes: request.memory_budget_bytes,
        },
        encoded_bytes: encoded.bytes.len() as u64,
        strips: 1,
    })
}
fn transfer_raster_error(error: vw_raster::RasterError) -> TransferError {
    match error {
        vw_raster::RasterError::Memory { estimated, budget } => {
            TransferError::Memory { estimated, budget }
        }
        vw_raster::RasterError::Dimensions { limit, .. } => TransferError::Dimensions {
            width: 0,
            height: 0,
            limit,
        },
        vw_raster::RasterError::Depth => TransferError::Depth,
        vw_raster::RasterError::Alpha => TransferError::Alpha,
        vw_raster::RasterError::EncodedLimit { .. } => TransferError::EncodedLimit,
        vw_raster::RasterError::ScratchLimit { .. } => TransferError::ScratchLimit,
        vw_raster::RasterError::Cancelled => TransferError::Cancelled,
        vw_raster::RasterError::Io => TransferError::Storage,
        vw_raster::RasterError::Allocation => TransferError::Backpressure,
        vw_raster::RasterError::Unsupported(_) => TransferError::Unsupported,
        vw_raster::RasterError::Color
        | vw_raster::RasterError::Metadata
        | vw_raster::RasterError::Codec
        | vw_raster::RasterError::OriginalRequired
        | vw_raster::RasterError::MissingAsset => TransferError::Metadata,
        _ => TransferError::Invalid,
    }
}

fn attach_file(
    state: &mut ProjectState,
    options: AttachFileOptions,
    cancel: &Cancellation,
) -> TransferResult<AttachedAsset> {
    cancel.check()?;
    memory(128 * 1024, options.memory_budget_bytes)?;
    validate_path(&options.source_path)?;
    limits(
        options.memory_budget_bytes,
        options.max_encoded_bytes,
        INPUT_LIMIT,
        options.max_scratch_bytes,
    )?;
    if options.lamport == 0 || options.now_ms < 0 {
        return Err(TransferError::Invalid);
    }
    let txn_id = Id::try_from(options.transaction_id)?;
    let device = DeviceId::try_from(options.device_id)?;
    if state.store()?.local_device()? != device {
        return Err(TransferError::Invalid);
    }
    let work = work_directory(&options.work_directory)?;
    let mut copied = tempfile::Builder::new()
        .prefix("vw-raster-attachment-")
        .tempfile_in(&work)?;
    let (mut source, size) =
        open_input(Path::new(&options.source_path), options.max_encoded_bytes)?;
    // The owned staging copy and BlobStore's no-clobber installation temporary
    // may coexist. Do not silently exceed the requested private scratch budget.
    if size
        .checked_mul(2)
        .is_none_or(|n| n > options.max_scratch_bytes)
    {
        return Err(TransferError::ScratchLimit);
    }
    let mut digest = blake3::Hasher::new();
    let mut remaining = size;
    let mut buffer = [0u8; 64 * 1024];
    while remaining > 0 {
        cancel.check()?;
        let count = (remaining.min(buffer.len() as u64)) as usize;
        source.read_exact(&mut buffer[..count])?;
        copied.write_all(&buffer[..count])?;
        digest.update(&buffer[..count]);
        remaining -= count as u64;
    }
    if source.read(&mut [0u8; 1])? != 0 {
        return Err(TransferError::Invalid);
    }
    drop(source);
    copied.as_file().sync_all()?;
    let id = AssetId::try_from(digest.finalize().to_hex().to_string())?;
    let mut definition = pb::AddAsset {
        asset_id: id.to_string(),
        orientation: 1,
        byte_size: size,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    };
    match options.kind {
        FileAssetKind::Mp4 => {
            validate_mp4(copied.as_file_mut(), size, cancel)?;
            definition.format = "mp4".into();
        }
        FileAssetKind::Image => {
            let bytes = read_bounded(
                copied.path(),
                options.max_encoded_bytes.min(options.memory_budget_bytes),
                cancel,
            )?;
            let mut scratch = tempfile::Builder::new()
                .prefix("vw-raster-source-")
                .tempfile_in(&work)?;
            let scratch_budget = options
                .max_scratch_bytes
                .checked_sub(size)
                .ok_or(TransferError::ScratchLimit)?;
            let (info, alpha) = with_source(
                &bytes,
                scratch.as_file_mut(),
                options.memory_budget_bytes,
                scratch_budget,
                cancel,
                |source| {
                    admit_scan(source, options.memory_budget_bytes)?;
                    let info = source.info().clone();
                    let mut alpha = false;
                    for y in (0..info.height).step_by(32) {
                        cancel.check()?;
                        let region = vw_raster::Region {
                            x: 0,
                            y,
                            width: info.width,
                            height: 32.min(info.height - y),
                        };
                        if source
                            .read_region(region, options.memory_budget_bytes, &|| {
                                cancel.is_cancelled()
                            })
                            .map_err(transfer_raster_error)?
                            .pixels
                            .has_alpha()
                        {
                            alpha = true;
                            break;
                        }
                    }
                    Ok((info, alpha))
                },
            )?;
            if info.source_asset != id {
                return Err(TransferError::Metadata);
            }
            let (width, height) = if info.orientation_applied >= 5 {
                (info.height, info.width)
            } else {
                (info.width, info.height)
            };
            definition.format = if vw_codec_os::is_heic(&bytes) {
                "heic"
            } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                "png"
            } else if bytes.starts_with(&[0xff, 0xd8]) {
                "jpeg"
            } else {
                "webp"
            }
            .into();
            definition.width = width;
            definition.height = height;
            definition.orientation = u32::from(info.orientation_applied);
            definition.bit_depth = u32::from(info.bit_depth);
            definition.has_alpha = alpha;
            definition.color_space = if info.icc.is_some() {
                "ICC"
            } else {
                "untagged"
            }
            .into();
            definition.icc_profile = info.icc.unwrap_or_default();
            scratch.close()?;
        }
    }
    vw_model::validate_asset(&definition)?;
    let previous = state.previous(&txn_id.to_proto())?;
    let txn = pb::Transaction {
        txn_id: Some(txn_id.to_proto()),
        project_id: Some(state.project()?.id.to_proto()),
        device_id: device.to_string(),
        base_revision: if let Some(previous) = previous {
            previous.base_revision.clone()
        } else {
            Some(state.store()?.revision()?)
        },
        created_at_wall_ms: options.now_ms,
        gesture_id: None,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device.to_string(),
                lamport: options.lamport,
            }),
            kind: Some(pb::op::Kind::AddAsset(definition.clone())),
        }],
    };
    if previous.is_some_and(|previous| previous != &txn) {
        return Err(TransferError::Invalid);
    }
    cancel.check()?;
    copied.as_file_mut().seek(SeekFrom::Start(0))?;
    if state.blobs.put_reader(copied.as_file_mut())? != id || state.blobs.verify(&id)? != size {
        return Err(TransferError::Storage);
    }
    // Accepted metadata never references an absent original. A cancellation or
    // failed transaction may retain an unreferenced immutable blob for cleanup.
    cancel.check()?;
    let revision = state.commit(&txn, &device, options.now_ms)?;
    Ok(AttachedAsset {
        asset_id: id.to_string(),
        byte_size: size,
        format: definition.format,
        revision,
    })
}
fn validate_mp4(file: &mut File, size: u64, cancel: &Cancellation) -> TransferResult<()> {
    let mut at = 0u64;
    let mut boxes = 0u32;
    let (mut ftyp, mut movie, mut media) = (false, false, false);
    while at < size {
        cancel.check()?;
        boxes += 1;
        if boxes > 4096 || size - at < 8 {
            return Err(TransferError::Metadata);
        }
        file.seek(SeekFrom::Start(at))?;
        let mut header = [0u8; 8];
        file.read_exact(&mut header)?;
        let short = u32::from_be_bytes(
            header[..4]
                .try_into()
                .map_err(|_| TransferError::Metadata)?,
        );
        let tag = &header[4..8];
        let (length, head) = if short == 1 {
            let mut extended = [0u8; 8];
            file.read_exact(&mut extended)?;
            (u64::from_be_bytes(extended), 16)
        } else if short == 0 {
            (size - at, 8)
        } else {
            (u64::from(short), 8)
        };
        if length < head || length > size - at {
            return Err(TransferError::Metadata);
        }
        if tag == b"ftyp" {
            if ftyp
                || at != 0
                || length < head + 8
                || length - head > 4096
                || (length - head) % 4 != 0
            {
                return Err(TransferError::Metadata);
            }
            let mut brands = vec![0u8; (length - head) as usize];
            file.read_exact(&mut brands)?;
            ftyp = brands
                .as_chunks::<4>()
                .0
                .iter()
                .enumerate()
                .any(|(i, brand)| {
                    i != 1
                        && matches!(
                            brand,
                            b"isom"
                                | b"iso2"
                                | b"iso3"
                                | b"iso4"
                                | b"iso5"
                                | b"iso6"
                                | b"mp41"
                                | b"mp42"
                                | b"avc1"
                                | b"M4V "
                        )
                });
            if !ftyp {
                return Err(TransferError::Metadata);
            }
        } else if tag == b"moov" {
            movie = length > head;
        } else if tag == b"mdat" {
            media = length > head;
        }
        at = at.checked_add(length).ok_or(TransferError::Metadata)?;
    }
    if !ftyp || !movie || !media {
        return Err(TransferError::Metadata);
    }
    Ok(())
}

pub(crate) fn open_input(path: &Path, limit: u64) -> TransferResult<(File, u64)> {
    check_components(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(TransferError::Invalid);
        }
    }
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(TransferError::Invalid);
    }
    if metadata.len() > limit {
        return Err(TransferError::EncodedLimit);
    }
    Ok((file, metadata.len()))
}

fn export_file(
    state: &ProjectState,
    options: FileExportOptions,
    cancel: &Cancellation,
) -> TransferResult<FileExportResult> {
    let work = work_directory(&options.work_directory)?;
    let destination = destination(&options.output_path, &work)?;
    let project = state.project()?;
    let revision = state.info()?;
    let document = Id::try_from(options.export.document_id.clone())?;
    let definition = &project
        .documents
        .get(&document)
        .ok_or(CoreError::Invalid)?
        .definition;
    let asset = AssetId::try_from(definition.primary_asset_id.clone())?;
    let source_path = state.blobs.path(&asset)?;
    let bytes = read_bounded(
        &source_path,
        INPUT_LIMIT.min(options.export.memory_budget_bytes),
        cancel,
    )?;
    if AssetId::hash(&bytes) != asset {
        return Err(TransferError::Storage);
    }
    let request = request(&options.export, definition, state.view_revision()?)?;
    let mut scratch = tempfile::Builder::new()
        .prefix("vw-raster-source-")
        .tempfile_in(&work)?;
    let mut output = tempfile::Builder::new()
        .prefix("vw-raster-output-")
        .tempfile_in(&work)?;
    let mut hash = blake3::Hasher::new();
    let report = {
        let mut sink = HashWriter {
            file: output.as_file_mut(),
            hash: &mut hash,
        };
        with_source(
            &bytes,
            scratch.as_file_mut(),
            request.memory_budget_bytes,
            options.max_scratch_bytes,
            cancel,
            |source| {
                validate_source_binding(source.info(), project, &asset)?;
                if !matches!(
                    request.format,
                    vw_raster::ExportFormat::Png8 | vw_raster::ExportFormat::Png16
                ) {
                    return export_buffered(
                        source, state, &document, &options, &request, cancel, &mut sink,
                    );
                }
                let mut adapter = SourceView(source);
                let limit = vw_raster::StreamLimits {
                    max_encoded_bytes: options.max_encoded_bytes,
                    max_strip_rows: 128,
                    max_pixels: 50_000_000,
                };
                if options.export.marked {
                    let assets = ResultAssets {
                        blobs: &state.blobs,
                        budget: request.memory_budget_bytes,
                        cancel,
                    };
                    vw_raster::export_document_png_to(
                        &mut adapter,
                        vw_raster::MarkedDocument {
                            project,
                            document: &document,
                            assets: &assets,
                            options: vw_raster::RenderOptions {
                                memory_budget_bytes: request.memory_budget_bytes,
                                include_guides: false,
                                assume_untagged_srgb: options.export.assume_untagged_srgb,
                            },
                        },
                        &request,
                        limit,
                        &mut sink,
                        &|| cancel.is_cancelled(),
                    )
                    .map_err(transfer_raster_error)
                } else {
                    vw_raster::export_png_to(&mut adapter, &request, limit, &mut sink, &|| {
                        cancel.is_cancelled()
                    })
                    .map_err(transfer_raster_error)
                }
            },
        )?
    };
    scratch.close()?;
    let metadata_json = serde_json::to_string(&report.metadata).map_err(|_| CoreError::Invalid)?;
    let mut response_bytes = crate::payload::info_bytes(&revision)?;
    crate::payload::accumulate(
        &mut response_bytes,
        metadata_json
            .len()
            .checked_mul(4)
            .ok_or(CoreError::Backpressure)?,
    )?;
    if output.as_file().metadata()?.len() != report.encoded_bytes {
        return Err(TransferError::Storage);
    }
    output.as_file().sync_all()?;
    cancel.check()?;
    // Only the owned temporary is cleaned on an error. persist_noclobber cannot
    // replace any existing file, and cancellation after publication keeps it.
    output
        .persist_noclobber(&destination)
        .map_err(|_| CoreError::Storage)?;
    sync_directory(&work)?;
    Ok(FileExportResult {
        encoded_bytes: report.encoded_bytes,
        blake3: hash.finalize().to_hex().to_string(),
        metadata_json,
        revision,
        estimated_peak_bytes: report.plan.estimated_peak_bytes,
    })
}
fn request(
    options: &ExportOptions,
    document: &pb::CreateDocument,
    revision: pb::Revision,
) -> Result<vw_raster::ExportRequest> {
    let format = match options.format {
        ImageFormat::Png8 => vw_raster::ExportFormat::Png8,
        ImageFormat::Png16 => vw_raster::ExportFormat::Png16,
        ImageFormat::Jpeg { quality } => vw_raster::ExportFormat::Jpeg { quality },
        ImageFormat::WebpLossless => vw_raster::ExportFormat::WebpLossless,
        ImageFormat::WebpLossy { quality } => vw_raster::ExportFormat::WebpLossy { quality },
    };
    let region = options
        .region
        .map(|r| {
            if [r.x, r.y, r.width, r.height]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0 || *v > f64::from(u32::MAX) || v.fract() != 0.0)
            {
                return Err(CoreError::Invalid);
            }
            Ok(vw_raster::Region {
                x: r.x as u32,
                y: r.y as u32,
                width: r.width as u32,
                height: r.height as u32,
            })
        })
        .transpose()?;
    Ok(vw_raster::ExportRequest {
        format,
        region,
        revision,
        alpha: options
            .matte_rgb
            .map_or(vw_raster::AlphaPolicy::Preserve, |m| {
                vw_raster::AlphaPolicy::Matte([(m >> 16) as u8, (m >> 8) as u8, m as u8])
            }),
        color: if options.convert_to_srgb {
            vw_raster::ColorPolicy::ConvertToSrgb {
                assume_untagged_srgb: options.assume_untagged_srgb,
            }
        } else {
            vw_raster::ColorPolicy::Preserve
        },
        allow_depth_reduction: options.allow_depth_reduction,
        memory_budget_bytes: options.memory_budget_bytes,
        capture_session: document
            .capture
            .as_ref()
            .map(|c| Id::from_proto(c.capture_session_id.as_ref()))
            .transpose()?,
        frame_id: document.capture.as_ref().map(|c| c.frame_id),
    })
}
fn with_source<R>(
    bytes: &[u8],
    scratch: &mut File,
    budget: u64,
    scratch_limit: u64,
    cancel: &Cancellation,
    work: impl FnOnce(&mut dyn RasterSource) -> TransferResult<R>,
) -> TransferResult<R> {
    let limits = SpoolLimits {
        decode: vw_raster::DecodeLimits {
            max_encoded_bytes: INPUT_LIMIT as usize,
            max_pixels: 50_000_000,
            max_memory_bytes: budget,
        },
        max_scratch_bytes: scratch_limit,
    };
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut source = PngSpool::decode(bytes, scratch, limits, &|| cancel.is_cancelled())
            .map_err(transfer_raster_error)?;
        return work(&mut source);
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        match JpegSpool::decode(bytes, scratch, limits, &|| cancel.is_cancelled()) {
            Ok(mut source) => return work(&mut source),
            Err(vw_raster::RasterError::Unsupported(_)) => {}
            Err(error) => return Err(transfer_raster_error(error)),
        }
    }
    // Keep smaller progressive JPEG/WebP compatible through the existing guarded
    // decoder; large cases retain its refusal. Account encoded bytes separately.
    let remaining = budget
        .checked_sub(bytes.len() as u64)
        .ok_or(CoreError::Backpressure)?;
    let image = crate::os_images::decode(
        bytes,
        vw_raster::DecodeLimits {
            max_memory_bytes: remaining,
            ..limits.decode
        },
        &|| cancel.is_cancelled(),
    )
    .map_err(transfer_raster_error)?;
    cancel.check()?;
    let retained_pixels = match &image.pixels {
        vw_raster::Pixels::Rgba8(p) => p.capacity() as u64,
        vw_raster::Pixels::Rgba16(p) => p.capacity() as u64 * 2,
    };
    memory(
        retained_pixels
            .checked_add(bytes.len() as u64)
            .and_then(|v| v.checked_add(profile_reserve(image.icc.is_some())))
            .ok_or(TransferError::Backpressure)?,
        budget,
    )?;
    let mut source = RetainedSource {
        inner: BorrowedSource::new(&image).map_err(transfer_raster_error)?,
        encoded: bytes.len() as u64,
    };
    work(&mut source)
}
struct SourceView<'a>(&'a mut dyn RasterSource);
impl RasterSource for SourceView<'_> {
    fn info(&self) -> &SourceInfo {
        self.0.info()
    }
    fn resident_bytes(&self) -> u64 {
        self.0.resident_bytes()
    }
    fn read_workspace_bytes(&self) -> u64 {
        self.0.read_workspace_bytes()
    }
    fn read_region(
        &mut self,
        r: vw_raster::Region,
        b: u64,
        c: &dyn Fn() -> bool,
    ) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        self.0.read_region(r, b, c)
    }
}
struct RetainedSource<'a> {
    inner: BorrowedSource<'a>,
    encoded: u64,
}
impl RasterSource for RetainedSource<'_> {
    fn info(&self) -> &SourceInfo {
        self.inner.info()
    }
    fn resident_bytes(&self) -> u64 {
        self.inner.resident_bytes() + self.encoded
    }
    fn read_workspace_bytes(&self) -> u64 {
        0
    }
    fn read_region(
        &mut self,
        r: vw_raster::Region,
        b: u64,
        c: &dyn Fn() -> bool,
    ) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        self.inner.read_region(
            r,
            b.checked_sub(self.encoded)
                .ok_or(vw_raster::RasterError::Memory {
                    estimated: self.encoded,
                    budget: b,
                })?,
            c,
        )
    }
}
struct ResultAssets<'a> {
    blobs: &'a vw_store::BlobStore,
    budget: u64,
    cancel: &'a Cancellation,
}
impl AssetResolver for ResultAssets<'_> {
    fn image(&self, asset: &str) -> std::result::Result<DecodedImage, vw_raster::RasterError> {
        let id = AssetId::try_from(asset.to_owned())
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let path = self
            .blobs
            .path(&id)
            .map_err(|_| vw_raster::RasterError::MissingAsset)?;
        let bytes =
            read_bounded(&path, INPUT_LIMIT.min(self.budget), self.cancel).map_err(|error| {
                match error {
                    CoreError::Cancelled => vw_raster::RasterError::Cancelled,
                    _ => vw_raster::RasterError::MissingAsset,
                }
            })?;
        if AssetId::hash(&bytes) != id {
            return Err(vw_raster::RasterError::MissingAsset);
        }
        let remaining = self
            .budget
            .checked_sub(bytes.len() as u64)
            .ok_or(vw_raster::RasterError::Allocation)?;
        crate::os_images::decode(
            &bytes,
            vw_raster::DecodeLimits {
                max_encoded_bytes: INPUT_LIMIT as usize,
                max_pixels: 50_000_000,
                max_memory_bytes: remaining,
            },
            &|| self.cancel.is_cancelled(),
        )
    }
}
struct HashWriter<'a> {
    file: &'a mut File,
    hash: &'a mut blake3::Hasher,
}
impl Write for HashWriter<'_> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(b)?;
        self.hash.update(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}
fn limits(memory: u64, encoded: u64, max_encoded: u64, scratch: u64) -> Result<()> {
    if memory == 0
        || memory > MEMORY_LIMIT
        || encoded == 0
        || encoded > max_encoded
        || scratch == 0
        || scratch > SCRATCH_LIMIT
    {
        Err(CoreError::Invalid)
    } else {
        Ok(())
    }
}
fn raster_error(error: vw_raster::RasterError) -> CoreError {
    match error {
        vw_raster::RasterError::Memory { .. }
        | vw_raster::RasterError::EncodedLimit { .. }
        | vw_raster::RasterError::ScratchLimit { .. }
        | vw_raster::RasterError::Allocation => CoreError::Backpressure,
        vw_raster::RasterError::Cancelled => CoreError::Cancelled,
        vw_raster::RasterError::Unsupported(_) => CoreError::Unsupported,
        vw_raster::RasterError::Io => CoreError::Storage,
        _ => CoreError::Raster,
    }
}
fn validate_path(path: &str) -> Result<()> {
    if path.is_empty()
        || path.len() > 32767
        || path.contains('\0')
        || !Path::new(path).is_absolute()
    {
        Err(CoreError::Invalid)
    } else {
        Ok(())
    }
}
fn check_components(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component);
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        let meta = std::fs::symlink_metadata(&current)?;
        if meta.file_type().is_symlink() {
            return Err(CoreError::Invalid);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(CoreError::Invalid);
            }
        }
    }
    Ok(())
}
fn work_directory(path: &str) -> Result<PathBuf> {
    validate_path(path)?;
    let path = Path::new(path);
    check_components(path)?;
    if !path.is_dir() {
        return Err(CoreError::Invalid);
    }
    Ok(path.canonicalize()?)
}
fn destination(path: &str, work: &Path) -> Result<PathBuf> {
    validate_path(path)?;
    let path = Path::new(path);
    let parent = path.parent().ok_or(CoreError::Invalid)?;
    check_components(parent)?;
    if parent.canonicalize()? != work {
        return Err(CoreError::Invalid);
    }
    let target = work.join(path.file_name().ok_or(CoreError::Invalid)?);
    match std::fs::symlink_metadata(&target) {
        Ok(_) => Err(CoreError::Invalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(target),
        Err(e) => Err(e.into()),
    }
}
fn read_bounded(path: &Path, limit: u64, cancel: &Cancellation) -> Result<Vec<u8>> {
    check_components(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
    }
    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(CoreError::Invalid);
        }
    }
    let size = metadata.len();
    if !metadata.is_file() || size == 0 {
        return Err(CoreError::Invalid);
    }
    if size > limit {
        return Err(CoreError::Backpressure);
    }
    let size = usize::try_from(size).map_err(|_| CoreError::Backpressure)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| CoreError::Backpressure)?;
    bytes.resize(size, 0);
    for part in bytes.chunks_mut(64 * 1024) {
        cancel.check()?;
        file.read_exact(part)?;
    }
    if file.read(&mut [0u8; 1])? != 0 {
        return Err(CoreError::Invalid);
    }
    cancel.check()?;
    Ok(bytes)
}
fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    fn temp() -> tempfile::TempDir {
        if cfg!(target_os = "android") {
            tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
        } else {
            tempfile::tempdir().unwrap()
        }
    }
    fn image() -> (Vec<u8>, vw_raster::Pixels) {
        let pixels = vw_raster::Pixels::Rgba16(
            (0..24u16)
                .flat_map(|i| [i * 997, 12345, 65535, if i % 3 == 0 { 0 } else { 65535 }])
                .collect(),
        );
        let source = DecodedImage {
            width: 6,
            height: 4,
            pixels: pixels.clone(),
            icc: None,
            source_asset: AssetId::hash(b"stream FFI synthetic"),
            original_available: true,
            orientation_applied: 1,
        };
        let request = vw_raster::ExportRequest {
            format: vw_raster::ExportFormat::Png16,
            region: None,
            revision: pb::Revision {
                host_seq: 0,
                state_hash: vec![0; 32],
            },
            alpha: vw_raster::AlphaPolicy::Preserve,
            color: vw_raster::ColorPolicy::Preserve,
            allow_depth_reduction: false,
            memory_budget_bytes: MEMORY_LIMIT,
            capture_session: None,
            frame_id: None,
        };
        (vw_raster::export(&source, &request).unwrap().bytes, pixels)
    }
    fn creation(root: &Path, source: &Path) -> CreateFileImageProject {
        let id = |n| Id::from_parts(1700000000000, [n; 10]).unwrap().to_string();
        CreateFileImageProject {
            path: root.join("project").to_string_lossy().into_owned(),
            project_id: id(1),
            document_id: id(2),
            layer_id: id(3),
            device_id: DeviceId::from_bytes([4; 16]).to_string(),
            title: "File raster fixture".into(),
            source_path: source.to_string_lossy().into_owned(),
            work_directory: root.to_string_lossy().into_owned(),
            now_ms: 0,
            memory_budget_bytes: MEMORY_LIMIT,
            max_encoded_bytes: INPUT_LIMIT,
            max_scratch_bytes: SCRATCH_LIMIT,
        }
    }
    fn exporting(root: &Path, doc: String) -> FileExportOptions {
        FileExportOptions {
            export: ExportOptions {
                document_id: doc,
                format: ImageFormat::Png16,
                marked: false,
                region: None,
                matte_rgb: None,
                convert_to_srgb: false,
                assume_untagged_srgb: false,
                allow_depth_reduction: false,
                memory_budget_bytes: MEMORY_LIMIT,
            },
            output_path: root.join("output.png").to_string_lossy().into_owned(),
            work_directory: root.to_string_lossy().into_owned(),
            max_encoded_bytes: OUTPUT_LIMIT,
            max_scratch_bytes: SCRATCH_LIMIT,
        }
    }
    fn no_scratch(root: &Path) {
        assert!(!std::fs::read_dir(root).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("vw-raster-")
        }));
    }
    #[tokio::test]
    async fn file_import_export_preserves_original_16bit_alpha_and_bound_revision() {
        let root = temp();
        let source = root.path().join("original.png");
        let (bytes, pixels) = image();
        std::fs::write(&source, &bytes).unwrap();
        let options = creation(root.path(), &source);
        let doc = options.document_id.clone();
        let path = options.path.clone();
        let project = create_file_image_project(options, Arc::new(Cancellation::new()))
            .await
            .unwrap();
        let before = project.info().await.unwrap();
        let exported = project
            .export_image_file(exporting(root.path(), doc), Arc::new(Cancellation::new()))
            .await
            .unwrap();
        let encoded = std::fs::read(root.path().join("output.png")).unwrap();
        assert_eq!(exported.encoded_bytes, encoded.len() as u64);
        assert_eq!(exported.blake3, blake3::hash(&encoded).to_hex().to_string());
        assert_eq!(exported.revision, before);
        assert_eq!(
            vw_raster::decode(&encoded, vw_raster::DecodeLimits::default())
                .unwrap()
                .pixels,
            pixels
        );
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        assert_eq!(
            vw_store::BlobStore::new(Path::new(&path))
                .unwrap()
                .read(&AssetId::hash(&bytes))
                .unwrap(),
            bytes
        );
        no_scratch(root.path());
        project.close().await.unwrap();
    }
    #[tokio::test]
    async fn export_limits_cancel_and_collision_preserve_targets_and_remove_owned_scratch() {
        let root = temp();
        let source = root.path().join("original.png");
        std::fs::write(&source, image().0).unwrap();
        let options = creation(root.path(), &source);
        let doc = options.document_id.clone();
        let project = create_file_image_project(options, Arc::new(Cancellation::new()))
            .await
            .unwrap();
        let mut options = exporting(root.path(), doc.clone());
        options.max_encoded_bytes = 1;
        assert!(matches!(
            project
                .export_image_file(options, Arc::new(Cancellation::new()))
                .await,
            Err(CoreError::Backpressure)
        ));
        assert!(!root.path().join("output.png").exists());
        no_scratch(root.path());
        let cancel = Arc::new(Cancellation::new());
        cancel.cancel();
        assert!(matches!(
            project
                .export_image_file(exporting(root.path(), doc.clone()), cancel)
                .await,
            Err(CoreError::Cancelled)
        ));
        no_scratch(root.path());
        std::fs::write(root.path().join("output.png"), b"owner sentinel").unwrap();
        assert!(
            project
                .export_image_file(exporting(root.path(), doc), Arc::new(Cancellation::new()))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(root.path().join("output.png")).unwrap(),
            b"owner sentinel"
        );
        no_scratch(root.path());
        project.close().await.unwrap();
    }
    #[tokio::test]
    async fn rejected_import_never_publishes_project_or_removes_original() {
        let root = temp();
        let source = root.path().join("original.png");
        let (bytes, _) = image();
        std::fs::write(&source, &bytes).unwrap();
        let mut options = creation(root.path(), &source);
        options.max_scratch_bytes = 1;
        assert!(matches!(
            create_file_image_project(options, Arc::new(Cancellation::new())).await,
            Err(CoreError::Backpressure)
        ));
        assert!(!root.path().join("project").exists());
        assert_eq!(std::fs::read(&source).unwrap(), bytes);
        no_scratch(root.path());
        let cancel = Arc::new(Cancellation::new());
        cancel.cancel();
        assert!(matches!(
            create_file_image_project(creation(root.path(), &source), cancel).await,
            Err(CoreError::Cancelled)
        ));
        assert!(!root.path().join("project").exists());
        no_scratch(root.path());
    }
    #[test]
    fn output_is_restricted_to_owned_work_directory() {
        let root = temp();
        let other = temp();
        assert!(
            destination(
                &other.path().join("outside.png").to_string_lossy(),
                root.path()
            )
            .is_err()
        );
        assert!(!other.path().join("outside.png").exists());
        #[cfg(unix)]
        {
            let path = root.path().join("linked");
            std::os::unix::fs::symlink(other.path(), &path).unwrap();
            assert!(work_directory(&path.to_string_lossy()).is_err());
        }
    }
}

//! Recovered giant_import publication/rebuild sequence. An ApprovedBuild is the
//! only producer capability; caller-selected output directories are not accepted.
//! Root retains PublishedProject until its budgeted facade constructor adopts it.
use super::{Error, Result, cache, input, io_error};
use crate::giant_images::source::{self, CapturedSource};
use crate::{
    Cancellation, WorkflowBinding,
    project::{ProjectState, SessionPermit},
};
use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
use vw_proto::v1 as pb;
use vw_store::{BlobStore, ProjectStore, RealFreeSpace};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

const PUBLISH_WORK_BYTES: u64 = 16 * 1024 * 1024;
const NEW_PROJECT_BYTES: u64 = 32 * 1024 * 1024;
const STAGING_OWNER_BYTES: u64 = 1024 * 1024;

struct StagingRetained {
    _source: Arc<input::PreparedSource>,
    _charge: MemoryCharge,
}

/// Accepted, reopened project resources and their retained shared-session charge.
/// Actual store/model allocations retire before the charge on every error path.
/// The root must preserve this owner until its budgeted facade adopts all fields.
pub(crate) struct PublishedProject {
    store: ProjectStore,
    blobs: BlobStore,
    permit: SessionPermit,
    charge: MemoryCharge,
}
impl PublishedProject {
    /// Split only immediately into one root payload owner, before fallible work.
    /// The root's from_store_budgeted hook must retain the charge through actual
    /// ProjectState destruction, including abandoned foreign replies.
    pub(crate) fn into_parts(self) -> (ProjectStore, BlobStore, SessionPermit, MemoryCharge) {
        (self.store, self.blobs, self.permit, self.charge)
    }
}

/// Internal lowering from root-selected publication fields, never a producer
/// directory or approval flag. Keep incoming strings bounded before cloning.
#[derive(Clone)]
pub(crate) struct GiantPublishOptions {
    pub path: String,
    pub project_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub device_id: String,
    pub title: String,
    pub now_ms: i64,
    pub disk_cache_bytes: u64,
}

fn validate_options(options: &GiantPublishOptions, disk_cache_bytes: u64) -> Result<()> {
    if options.path.is_empty()
        || options.path.len() > 8192
        || !Path::new(&options.path).is_absolute()
        || options.project_id.len() != 36
        || options.document_id.len() != 36
        || options.layer_id.len() != 36
        || options.device_id.len() > 64
        || options.title.len() > 4096
        || !(0..(1i64 << 48)).contains(&options.now_ms)
        || !(1..=super::MAX_DISK_BYTES).contains(&options.disk_cache_bytes)
        || options.disk_cache_bytes != disk_cache_bytes
    {
        return Err(Error::Invalid);
    }
    Id::try_from(options.project_id.clone()).map_err(|_| Error::Invalid)?;
    Id::try_from(options.document_id.clone()).map_err(|_| Error::Invalid)?;
    Id::try_from(options.layer_id.clone()).map_err(|_| Error::Invalid)?;
    DeviceId::try_from(options.device_id.clone()).map_err(|_| Error::Invalid)?;
    Ok(())
}

/// Sibling staging is disposable until the no-clobber rename accepts it. After
/// that commit point, errors/cancelled foreign delivery preserve the complete
/// project at its final destination for an explicit reopen.
pub(super) fn publish(
    build: &cache::ApprovedBuild,
    options: GiantPublishOptions,
    closed: &AtomicBool,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<PublishedProject> {
    source::live(closed, cancel)?;
    let _work = source::reserve(budget, MemoryKind::CodecWorkspace, PUBLISH_WORK_BYTES)?;
    // Reserve before creating the model/store. This charge must move into the
    // actual returned ProjectState, not retire with this temporary call frame.
    let project_charge = source::reserve(budget, MemoryKind::CodecWorkspace, NEW_PROJECT_BYTES)?;
    validate_options(&options, build.disk_cache_bytes)?;
    if !build._manifest_charge.belongs_to(budget) || build.manifest.source() != &build.source.source
    {
        return Err(Error::Invalid);
    }
    build.source.verify(closed, cancel, budget)?;
    cache::verify_output(build, closed, None, cancel, budget)?;
    let permit = SessionPermit::acquire().map_err(Error::from)?;
    let (project, device) = initial_project(&options, build)?;
    let binding = WorkflowBinding {
        project_id: options.project_id.clone(),
        document_id: options.document_id.clone(),
        host_seq: 0,
        state_hash: project
            .state_hash()
            .map_err(|_| Error::Invalid)?
            .to_string(),
    };
    let requested = Path::new(&options.path);
    let parent = input::checked_directory(requested.parent().ok_or(Error::Invalid)?)?;
    let destination = parent.join(requested.file_name().ok_or(Error::Invalid)?);
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => return Err(Error::Invalid),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    if build.source.required_work_bytes > build.source.max_work_bytes {
        return Err(Error::Limit);
    }
    input::admit_space(&parent, build.source.required_work_bytes, &RealFreeSpace)?;
    let cleanup_charge = source::reserve(budget, MemoryKind::Transfer, STAGING_OWNER_BYTES)?;
    let temporary = tempfile::Builder::new()
        .prefix(".vw-giant-create-")
        .tempdir_in(&parent)
        .map_err(io_error)?;
    let mut staging = super::cleanup::OwnedDirectory::new(
        temporary,
        StagingRetained {
            _source: build.source.clone(),
            _charge: cleanup_charge,
        },
    );
    let candidate = staging.path().join("project");
    {
        let store = ProjectStore::create(&candidate, project, device, options.now_ms)
            .map_err(source::store_error)?;
        let blobs = BlobStore::new(&candidate).map_err(source::store_error)?;
        install_original(&blobs, &candidate, build, closed, cancel, budget)?;
        cache::install(
            &candidate,
            build,
            RealFreeSpace,
            closed,
            None,
            cancel,
            budget,
        )?;
        check_created_binding(&store, &binding)?;
        store.integrity_check().map_err(source::store_error)?;
        // Release the actual SQLite writer/lock before Windows directory rename.
    }
    build.source.verify(closed, cancel, budget)?;
    source::live(closed, cancel)?;
    crate::creation::sync_directory(&candidate).map_err(Error::from)?;
    source::live(closed, cancel)?;
    crate::creation::publish(&candidate, &destination).map_err(Error::from)?;
    // Accepted directory is outside staging now. No error path removes it.
    crate::creation::sync_directory(&parent).map_err(Error::from)?;
    // Failed cleanup retains this exact directory, source/permit and charge in
    // quarantine. The root bounds this build to one publication attempt.
    staging.cleanup()?;
    source::live(closed, cancel)?;
    let store = ProjectStore::open(&destination).map_err(source::store_error)?;
    let blobs = BlobStore::new(&destination).map_err(source::store_error)?;
    let installed = installed_source(&blobs, &destination, build)?;
    installed.verify(closed, cancel, budget)?;
    check_created_binding(&store, &binding)?;
    source::live(closed, cancel)?;
    // Registration is at the final root, after exact original verification and
    // the complete project/document/hostSeq/stateHash fence under its writer lock.
    cache::register(&destination, build, budget, cancel)?;
    // Infallible owner construction keeps these allocations charged while the
    // root performs its separately owned facade/budgeted-constructor adoption.
    Ok(PublishedProject {
        store,
        blobs,
        permit,
        charge: project_charge,
    })
}

fn initial_project(
    options: &GiantPublishOptions,
    build: &cache::ApprovedBuild,
) -> Result<(Project, DeviceId)> {
    let input = &build.source;
    let project_id = Id::try_from(options.project_id.clone()).map_err(|_| Error::Invalid)?;
    let document = Id::try_from(options.document_id.clone()).map_err(|_| Error::Invalid)?;
    let layer = Id::try_from(options.layer_id.clone()).map_err(|_| Error::Invalid)?;
    let device = DeviceId::try_from(options.device_id.clone()).map_err(|_| Error::Invalid)?;
    let asset = AssetId::try_from(crate::giant_images::tiles::hex(&input.info.asset_id))
        .map_err(|_| Error::Invalid)?;
    let oriented = if input.source.orientation >= 5 {
        (input.info.raw_height, input.info.raw_width)
    } else {
        (input.info.raw_width, input.info.raw_height)
    };
    if asset.bytes() != input.source.original
        || oriented != (input.source.width, input.source.height)
        || input.info.encoded_bytes != input.source.original_bytes
        || input.info.orientation != input.source.orientation
        || input.info.format != input.format
    {
        return Err(Error::Stale);
    }
    let mut project = Project::new(project_id, options.title.clone(), device.clone());
    project.assets.insert(
        asset,
        pb::AddAsset {
            asset_id: crate::giant_images::tiles::hex(&input.info.asset_id),
            format: input.format.into(),
            width: input.info.raw_width,
            height: input.info.raw_height,
            orientation: u32::from(input.source.orientation),
            bit_depth: 8,
            has_alpha: false,
            color_space: "untagged".into(),
            icc_profile: Vec::new(),
            byte_size: input.source.original_bytes,
            source: "import".into(),
            captured_at_ms: 0,
            // PreparedSource exists only after explicit untagged-sRGB consent.
            // Provenance describes the derived cache's interpretation; the
            // immutable original retains its actual untagged profile/depth.
            metadata_json: provenance(build.platform, &build.manifest.identity())?,
        },
    );
    project.documents.insert(
        document.clone(),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(document.to_proto()),
                kind: pb::DocumentKind::Image as i32,
                capture: None,
                schema_version: 1,
                title: options.title.clone(),
                primary_asset_id: crate::giant_images::tiles::hex(&input.info.asset_id),
            },
            pages: Vec::new(),
            created_at_ms: options.now_ms,
        },
    );
    project.layers.insert(
        layer.clone(),
        Layer {
            definition: pb::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(document.to_proto()),
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
    project.validate().map_err(|_| Error::Invalid)?;
    Ok((project, device))
}

fn provenance(platform: &str, manifest: &[u8; 32]) -> Result<String> {
    if platform.is_empty() || platform.len() > 128 {
        return Err(Error::Invalid);
    }
    // Existing asset metadata deliberately accepts only flat descriptive
    // strings. Do not reopen its privacy boundary with a nested cache object.
    let value = serde_json::json!({
        "software": format!("VisualWorkbench giant derived cache; producer={platform}; manifest_hash={}",crate::giant_images::tiles::hex(manifest)),
        "color_profile": "derived_profile=sRGB; untagged_srgb_consent=true; source_conversion=false",
    });
    let json = serde_json::to_string(&value).map_err(|_| Error::Invalid)?;
    if json.len() > 2048 {
        return Err(Error::Limit);
    }
    Ok(json)
}

fn check_created_binding(store: &ProjectStore, expected: &WorkflowBinding) -> Result<()> {
    let project = store.project();
    let document = Id::try_from(expected.document_id.clone()).map_err(|_| Error::Invalid)?;
    let revision = store.revision().map_err(source::store_error)?;
    if project.id.as_str() != expected.project_id
        || !project.documents.contains_key(&document)
        || revision.host_seq != expected.host_seq
        || project.state_hash().map_err(|_| Error::Invalid)?.as_str() != expected.state_hash
    {
        return Err(Error::Stale);
    }
    Ok(())
}

fn installed_source(
    blobs: &BlobStore,
    root: &Path,
    build: &cache::ApprovedBuild,
) -> Result<CapturedSource> {
    let asset = AssetId::try_from(crate::giant_images::tiles::hex(&build.source.info.asset_id))
        .map_err(|_| Error::Invalid)?;
    Ok(CapturedSource {
        root: root.to_path_buf(),
        path: blobs.path(&asset).map_err(source::store_error)?,
        source: build.source.source.clone(),
        format: build.source.format,
    })
}

fn install_original(
    blobs: &BlobStore,
    root: &Path,
    build: &cache::ApprovedBuild,
    closed: &AtomicBool,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<()> {
    source::live(closed, cancel)?;
    let mut input = source::regular(&build.source.info.source_path).map_err(Error::from)?;
    if input.metadata().map_err(io_error)?.len() != build.source.source.original_bytes {
        return Err(Error::Stale);
    }
    input.seek(SeekFrom::Start(0)).map_err(io_error)?;
    let mut reader = CancelledRead {
        input,
        closed,
        cancel,
        remaining: build.source.source.original_bytes,
        failure: None,
    };
    let asset = match blobs.put_reader(&mut reader) {
        Ok(asset) => asset,
        Err(error) => {
            return Err(match reader.failure.take() {
                Some(failure) => failure,
                None => source::store_error(error),
            });
        }
    };
    if reader.remaining != 0 || asset.bytes() != build.source.source.original {
        return Err(Error::Stale);
    }
    // The source was copied through BlobStore's bounded 64KiB buffer and the
    // installed content address was matched. Never read the whole original Vec.
    let installed = installed_source(blobs, root, build)?;
    installed.verify(closed, cancel, budget)
}

struct CancelledRead<'a, R: Read> {
    input: R,
    closed: &'a AtomicBool,
    cancel: &'a Cancellation,
    remaining: u64,
    failure: Option<Error>,
}
impl<R: Read> Read for CancelledRead<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if let Err(error) = source::live(self.closed, self.cancel) {
            self.failure = Some(error);
            return Err(std::io::Error::other("giant input stopped"));
        }
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.remaining == 0 {
            let mut sentinel = [0];
            if self.input.read(&mut sentinel)? != 0 {
                self.failure = Some(Error::Stale);
                return Err(std::io::Error::other("giant input length"));
            }
            return Ok(0);
        }
        let remaining = match usize::try_from(self.remaining) {
            Ok(remaining) => remaining,
            Err(_) => usize::MAX,
        };
        let cap = bytes.len().min(remaining);
        let count = self.input.read(&mut bytes[..cap])?;
        if count == 0 {
            self.failure = Some(Error::Stale);
            return Err(std::io::Error::other("giant input length"));
        }
        self.remaining -= count as u64;
        Ok(count)
    }
}

/// Invoke only inside the one already-enqueued project worker job. The outer
/// producer owner retains its Arc<ApprovedBuild> and joins this actual job even
/// after foreign future loss; no project-worker await is performed from here.
pub(super) fn rebuild_in_project_worker(
    state: &mut ProjectState,
    build: &cache::ApprovedBuild,
    project_closed: &AtomicBool,
    binding: &WorkflowBinding,
    closed: &AtomicBool,
    cancel: &Cancellation,
    budget: &SessionBudget,
) -> Result<()> {
    source::live(closed, cancel)?;
    source::live(project_closed, cancel)?;
    let _work = source::reserve(budget, MemoryKind::CodecWorkspace, PUBLISH_WORK_BYTES)?;
    if !build._manifest_charge.belongs_to(budget) {
        return Err(Error::Invalid);
    }
    let before = source::capture(state, binding, project_closed, cancel, budget)?;
    same_source(&before, build)?;
    before.verify_owned(project_closed, closed, cancel, budget)?;
    cache::install(
        &before.root,
        build,
        RealFreeSpace,
        closed,
        Some(project_closed),
        cancel,
        budget,
    )?;
    // Capture repeats the entire visible binding, rather than only the original
    // hash. Running this fence and registration in the same project job prevents
    // another queued project edit from slipping between them.
    let after = source::capture(state, binding, project_closed, cancel, budget)?;
    same_source(&after, build)?;
    if before.root != after.root || before.path != after.path {
        return Err(Error::Stale);
    }
    after.verify_owned(project_closed, closed, cancel, budget)?;
    source::live(closed, cancel)?;
    source::live(project_closed, cancel)?;
    cache::register(&after.root, build, budget, cancel)
}

fn same_source(captured: &CapturedSource, build: &cache::ApprovedBuild) -> Result<()> {
    if captured.source != build.source.source
        || captured.format != build.source.format
        || build.manifest.source() != &captured.source
    {
        return Err(Error::Stale);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Cursor, sync::atomic::Ordering};

    fn options() -> GiantPublishOptions {
        GiantPublishOptions {
            path: if cfg!(windows) {
                r"C:\giant-new-project"
            } else {
                "/giant-new-project"
            }
            .into(),
            project_id: "018bcfe5-6800-7000-8000-000000000001".into(),
            document_id: "018bcfe5-6800-7000-8000-000000000002".into(),
            layer_id: "018bcfe5-6800-7000-8000-000000000003".into(),
            device_id: DeviceId::from_bytes([3; 16]).to_string(),
            title: "Fixture".into(),
            now_ms: 0,
            disk_cache_bytes: 128 * 1024 * 1024,
        }
    }
    #[test]
    fn publish_options_are_bounded_and_cannot_override_the_approved_disk_policy() {
        let mut options = options();
        let disk = options.disk_cache_bytes;
        assert!(validate_options(&options, disk).is_ok());
        assert!(matches!(
            validate_options(&options, disk - 1),
            Err(Error::Invalid)
        ));
        options.title = "x".repeat(4097);
        assert!(matches!(
            validate_options(&options, disk),
            Err(Error::Invalid)
        ));
        options.title.clear();
        options.now_ms = 1i64 << 48;
        assert!(validate_options(&options, disk).is_err());
    }
    #[test]
    fn provenance_records_explicit_interpretation_and_exact_runner_manifest() -> Result<()> {
        let encoded = provenance("windows-libvips-8.18.7", &[7; 32])?;
        let value: serde_json::Value =
            serde_json::from_str(&encoded).map_err(|_| Error::Invalid)?;
        let color = value["color_profile"].as_str().ok_or(Error::Invalid)?;
        let software = value["software"].as_str().ok_or(Error::Invalid)?;
        assert!(color.contains("untagged_srgb_consent=true"));
        assert!(color.contains("source_conversion=false"));
        assert!(color.contains("derived_profile=sRGB"));
        assert!(software.contains("producer=windows-libvips-8.18.7"));
        assert!(software.contains(&"07".repeat(32)));
        assert_eq!(value.as_object().ok_or(Error::Invalid)?.len(), 2);
        assert!(encoded.len() <= 2048);
        assert!(provenance(&"x".repeat(129), &[7; 32]).is_err());
        Ok(())
    }
    #[test]
    fn streamed_original_refuses_short_and_long_sources() {
        let closed = AtomicBool::new(false);
        let cancel = Cancellation::new();
        for (input, expected) in [(b"ab".as_slice(), 3), (b"abcd".as_slice(), 3)] {
            let mut reader = CancelledRead {
                input: Cursor::new(input),
                closed: &closed,
                cancel: &cancel,
                remaining: expected,
                failure: None,
            };
            let mut bounded = Vec::new();
            assert!(reader.read_to_end(&mut bounded).is_err());
            assert!(matches!(reader.failure, Some(Error::Stale)));
            assert!(bounded.len() <= 3);
        }
    }
    #[test]
    fn streamed_original_checks_close_and_cancellation_for_each_chunk() {
        let closed = AtomicBool::new(false);
        let cancel = Cancellation::new();
        let mut reader = CancelledRead {
            input: Cursor::new(b"abc"),
            closed: &closed,
            cancel: &cancel,
            remaining: 3,
            failure: None,
        };
        let mut byte = [0];
        assert_eq!(reader.read(&mut byte).ok(), Some(1));
        closed.store(true, Ordering::Release);
        assert!(reader.read(&mut byte).is_err());
        assert!(matches!(reader.failure, Some(Error::Closed)));
        closed.store(false, Ordering::Release);
        cancel.cancel();
        assert!(reader.read(&mut byte).is_err());
        assert!(matches!(reader.failure, Some(Error::Cancelled)));
    }
    #[test]
    fn streamed_original_never_truncates_a_successful_blob() {
        let closed = AtomicBool::new(false);
        let cancel = Cancellation::new();
        let mut reader = CancelledRead {
            input: Cursor::new(b"abc"),
            closed: &closed,
            cancel: &cancel,
            remaining: 3,
            failure: None,
        };
        let mut output = Vec::new();
        assert_eq!(reader.read_to_end(&mut output).ok(), Some(3));
        assert_eq!(output, b"abc");
        assert_eq!(reader.remaining, 0);
        assert!(reader.failure.is_none());
    }
}

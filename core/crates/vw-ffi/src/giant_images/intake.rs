//! Public owned-input facade. Paths are request-only, never returned metadata.
use super::{
    GiantBuiltCache, GiantImageError as Error, GiantImageResult as Result, PreparedOriginal,
    adopt_published_project, producer,
};
use crate::{Cancellation, ProjectSession, WorkflowBinding};
use std::{path::Path, sync::Arc};

#[derive(uniffi::Record)]
pub struct GiantOriginalOptions {
    pub source_path: String,
    pub work_directory: String,
    pub max_original_bytes: u64,
    pub max_work_disk_bytes: u64,
    pub assume_untagged_srgb: bool,
}
#[derive(Clone, uniffi::Record)]
pub struct GiantOriginalInfo {
    pub asset_id: String,
    pub sha256: String,
    pub encoded_bytes: u64,
    pub raw_width: u32,
    pub raw_height: u32,
    pub width: u32,
    pub height: u32,
    pub orientation: u8,
    pub format: String,
}
#[derive(uniffi::Record)]
pub struct GiantCacheBuildOptions {
    pub disk_cache_bytes: u64,
    pub timeout_ms: u64,
    pub assume_untagged_srgb: bool,
}
#[derive(uniffi::Record)]
pub struct GiantProjectOptions {
    pub path: String,
    pub project_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub device_id: String,
    pub title: String,
    pub now_ms: i64,
    pub disk_cache_bytes: u64,
}

#[derive(uniffi::Object)]
pub struct GiantPickedOriginal {
    inner: Arc<producer::PickedOriginal>,
}

/// Open local filesystem paths only. A content URI requires a separately
/// admitted, bounded staging adapter. This call does not start preparation.
#[uniffi::export]
pub fn open_giant_original(
    options: GiantOriginalOptions,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<GiantPickedOriginal>> {
    for path in [&options.source_path, &options.work_directory] {
        if path.is_empty()
            || path.len() > 8192
            || path.contains('\0')
            || !Path::new(path).is_absolute()
        {
            return Err(Error::Invalid);
        }
    }
    let inner = producer::pick_path(
        Path::new(&options.source_path),
        Path::new(&options.work_directory),
        producer::GiantInputLimits {
            max_original_bytes: options.max_original_bytes,
            max_work_disk_bytes: options.max_work_disk_bytes,
            assume_untagged_srgb: options.assume_untagged_srgb,
        },
        crate::session_resources::shared_budget()?,
        &cancellation,
    )?;
    Ok(Arc::new(GiantPickedOriginal { inner }))
}

#[uniffi::export]
impl GiantPickedOriginal {
    pub async fn prepare(&self, cancellation: Arc<Cancellation>) -> Result<Arc<PreparedOriginal>> {
        self.inner.prepare(cancellation).await
    }
    pub async fn close(&self) -> Result<()> {
        self.inner.close_worker().await
    }
}

#[uniffi::export]
impl PreparedOriginal {
    pub fn describe_original(&self) -> GiantOriginalInfo {
        let value = self.describe();
        GiantOriginalInfo {
            asset_id: blake3::Hash::from_bytes(value.asset_id)
                .to_hex()
                .to_string(),
            sha256: value
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            encoded_bytes: value.encoded_bytes,
            raw_width: value.raw_width,
            raw_height: value.raw_height,
            width: value.width,
            height: value.height,
            orientation: value.orientation,
            format: value.format,
        }
    }
    pub async fn build_cache(
        &self,
        options: GiantCacheBuildOptions,
        cancellation: Arc<Cancellation>,
    ) -> Result<Arc<GiantBuiltCache>> {
        // The foreign caller cannot select or approve a platform runtime.
        #[cfg(windows)]
        let platform = producer::GiantProducerPlatform::WindowsPinnedVips;
        #[cfg(not(windows))]
        let platform = producer::GiantProducerPlatform::AndroidRegionJpeg;
        self.build_approved(
            producer::GiantProducerOptions {
                platform,
                disk_cache_bytes: options.disk_cache_bytes,
                timeout_ms: options.timeout_ms,
                assume_untagged_srgb: options.assume_untagged_srgb,
            },
            cancellation,
        )
        .await
    }
    pub async fn close(&self) -> Result<()> {
        self.close_worker().await
    }
}

#[uniffi::export]
impl GiantBuiltCache {
    pub async fn publish_project(
        &self,
        options: GiantProjectOptions,
        cancellation: Arc<Cancellation>,
    ) -> Result<Arc<ProjectSession>> {
        let published = self
            .publish(
                producer::GiantPublishOptions {
                    path: options.path,
                    project_id: options.project_id,
                    document_id: options.document_id,
                    layer_id: options.layer_id,
                    device_id: options.device_id,
                    title: options.title,
                    now_ms: options.now_ms,
                    disk_cache_bytes: options.disk_cache_bytes,
                },
                cancellation,
            )
            .await?;
        adopt_published_project(published).map_err(Error::from)
    }
    pub async fn close(&self) -> Result<()> {
        self.close_worker().await
    }
}

#[uniffi::export]
impl ProjectSession {
    pub async fn rebuild_giant_image(
        &self,
        built: Arc<GiantBuiltCache>,
        binding: WorkflowBinding,
        cancellation: Arc<Cancellation>,
    ) -> Result<()> {
        self.check_open().map_err(Error::from)?;
        built
            .rebuild(
                self.worker.clone(),
                self.closed.clone(),
                binding,
                cancellation,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;
    const OWNER: u64 = 4 * 1024 * 1024;
    fn token() -> Arc<Cancellation> {
        Arc::new(Cancellation::new())
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }
    struct Fixture {
        _root: tempfile::TempDir,
        source: PathBuf,
        work: PathBuf,
        bytes: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().join("selected.png");
            let work = root.path().join("work");
            std::fs::create_dir(&work).unwrap();
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
                encoder.set_color(png::ColorType::Rgb);
                encoder.set_depth(png::BitDepth::Eight);
                encoder
                    .write_header()
                    .unwrap()
                    .write_image_data(&[21, 42, 84])
                    .unwrap();
            }
            std::fs::write(&source, &bytes).unwrap();
            Self {
                _root: root,
                source,
                work,
                bytes,
            }
        }
        fn options(&self) -> GiantOriginalOptions {
            GiantOriginalOptions {
                source_path: self.source.to_string_lossy().into_owned(),
                work_directory: self.work.to_string_lossy().into_owned(),
                max_original_bytes: 4 * 1024 * 1024,
                max_work_disk_bytes: 512 * 1024 * 1024,
                assume_untagged_srgb: true,
            }
        }
        fn children(&self) -> usize {
            std::fs::read_dir(&self.work).unwrap().count()
        }
    }
    #[test]
    fn public_input_saturation_refuses_before_missing_source_or_work_access() {
        runtime().block_on(async {
            let fixture = Fixture::new();
            let pool = crate::shared_session_resources().unwrap();
            let before = pool.used_bytes();
            let first = open_giant_original(fixture.options(), token()).unwrap();
            let second = open_giant_original(fixture.options(), token()).unwrap();
            assert_eq!(pool.used_bytes(), before + 2 * OWNER);
            let mut missing = fixture.options();
            missing.source_path.push_str(".missing");
            missing.work_directory.push_str(".missing");
            assert!(matches!(
                open_giant_original(missing, token()),
                Err(Error::Backpressure)
            ));
            assert_eq!(fixture.children(), 2);
            first.close().await.unwrap();
            second.close().await.unwrap();
            assert_eq!(fixture.children(), 0);
            assert_eq!(pool.used_bytes(), before);
            assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.bytes);
        });
    }
    #[test]
    fn public_input_uses_the_existing_global_foreign_memory_pool_before_io() {
        let fixture = Fixture::new();
        let pool = crate::shared_session_resources().unwrap();
        let before = pool.used_bytes();
        let charge = pool
            .reserve_foreign(pool.limit_bytes() - before - OWNER + 1)
            .unwrap();
        let mut options = fixture.options();
        options.source_path.push_str(".missing");
        assert!(matches!(
            open_giant_original(options, token()),
            Err(Error::Memory { .. })
        ));
        assert_eq!(fixture.children(), 0);
        charge.close().unwrap();
        assert_eq!(pool.used_bytes(), before);
    }
    #[test]
    fn public_prepare_transfers_one_owner_and_hides_paths_from_metadata() {
        runtime().block_on(async {
            let fixture = Fixture::new();
            let pool = crate::shared_session_resources().unwrap();
            let before = pool.used_bytes();
            let picked = open_giant_original(fixture.options(), token()).unwrap();
            let prepared = picked.prepare(token()).await.unwrap();
            assert!(matches!(
                picked.prepare(token()).await,
                Err(Error::Backpressure)
            ));
            picked.close().await.unwrap();
            assert_eq!(pool.used_bytes(), before + OWNER);
            let info = prepared.describe_original();
            assert_eq!(
                (
                    info.raw_width,
                    info.raw_height,
                    info.width,
                    info.height,
                    info.orientation
                ),
                (1, 1, 1, 1, 1)
            );
            assert_eq!(info.format, "png");
            assert_eq!(info.encoded_bytes, fixture.bytes.len() as u64);
            assert_eq!(
                info.asset_id,
                blake3::hash(&fixture.bytes).to_hex().to_string()
            );
            let digest = Sha256::digest(&fixture.bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            assert_eq!(info.sha256, digest);
            assert!(matches!(
                prepared
                    .build_cache(
                        GiantCacheBuildOptions {
                            disk_cache_bytes: 1,
                            timeout_ms: 1000,
                            assume_untagged_srgb: false
                        },
                        token()
                    )
                    .await,
                Err(Error::Unsupported)
            ));
            assert!(matches!(
                prepared
                    .build_cache(
                        GiantCacheBuildOptions {
                            disk_cache_bytes: 0,
                            timeout_ms: 1000,
                            assume_untagged_srgb: true
                        },
                        token()
                    )
                    .await,
                Err(Error::Limit)
            ));
            #[cfg(not(any(windows, target_os = "android")))]
            assert!(matches!(
                prepared
                    .build_cache(
                        GiantCacheBuildOptions {
                            disk_cache_bytes: 32 * 1024 * 1024,
                            timeout_ms: 1000,
                            assume_untagged_srgb: true
                        },
                        token()
                    )
                    .await,
                Err(Error::Unsupported)
            ));
            prepared.close().await.unwrap();
            prepared.close().await.unwrap();
            assert_eq!(fixture.children(), 0);
            assert_eq!(pool.used_bytes(), before);
            assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.bytes);
        });
    }
    #[test]
    fn cancelled_or_malformed_input_retires_actual_work_without_removing_original() {
        runtime().block_on(async {
            let fixture = Fixture::new();
            let before = crate::shared_session_resources().unwrap().used_bytes();
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                open_giant_original(fixture.options(), cancel.clone()),
                Err(Error::Cancelled)
            ));
            assert_eq!(fixture.children(), 0);
            let picked = open_giant_original(fixture.options(), token()).unwrap();
            assert!(matches!(
                picked.prepare(cancel).await,
                Err(Error::Cancelled)
            ));
            picked.close().await.unwrap();
            std::fs::write(&fixture.source, [0u8; 64]).unwrap();
            let malformed = open_giant_original(fixture.options(), token()).unwrap();
            assert!(malformed.prepare(token()).await.is_err());
            malformed.close().await.unwrap();
            assert_eq!(fixture.children(), 0);
            assert_eq!(std::fs::read(&fixture.source).unwrap(), [0; 64]);
            assert_eq!(
                crate::shared_session_resources().unwrap().used_bytes(),
                before
            );
        });
    }
    #[test]
    fn invalid_uri_consent_and_limits_do_not_create_work() {
        let fixture = Fixture::new();
        let mut uri = fixture.options();
        uri.source_path = "content://synthetic/original".into();
        assert!(matches!(
            open_giant_original(uri, token()),
            Err(Error::Invalid)
        ));
        let mut consent = fixture.options();
        consent.assume_untagged_srgb = false;
        assert!(matches!(
            open_giant_original(consent, token()),
            Err(Error::Unsupported)
        ));
        let mut limit = fixture.options();
        limit.max_original_bytes = 0;
        assert!(matches!(
            open_giant_original(limit, token()),
            Err(Error::Limit)
        ));
        assert_eq!(fixture.children(), 0);
        let picked = open_giant_original(fixture.options(), token()).unwrap();
        drop(picked);
        assert_eq!(fixture.children(), 0);
    }
    #[cfg(unix)]
    #[test]
    fn symlink_original_and_work_parent_refuse_without_following() {
        let fixture = Fixture::new();
        let link = fixture._root.path().join("source-link");
        std::os::unix::fs::symlink(&fixture.source, &link).unwrap();
        let mut options = fixture.options();
        options.source_path = link.to_string_lossy().into_owned();
        assert!(open_giant_original(options, token()).is_err());
        let work_link = fixture._root.path().join("work-link");
        std::os::unix::fs::symlink(&fixture.work, &work_link).unwrap();
        let mut options = fixture.options();
        options.work_directory = work_link.to_string_lossy().into_owned();
        assert!(open_giant_original(options, token()).is_err());
        assert_eq!(fixture.children(), 0);
        assert_eq!(std::fs::read(&fixture.source).unwrap(), fixture.bytes);
    }
}

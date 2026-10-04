//! Capture provenance is admitted before the existing staged image importer
//! publishes a project. No alternate SQLite/blob publication path lives here.
use crate::{Cancellation, CoreError, CreateFileImageProject, ProjectSession, Result};
use std::sync::Arc;
use vw_model::{AssetId, Id};
use vw_proto::v1;

#[derive(Clone, Debug, uniffi::Record)]
pub struct CaptureImportDescriptor {
    pub explicit_owner_action: bool,
    pub capture_session_id: String,
    pub frame_id: u64,
    pub geometry_revision: u32,
    pub platform: String,
    pub source_kind: String,
    pub physical_x: i32,
    pub physical_y: i32,
    pub width: u32,
    pub height: u32,
    pub dpi_scale: f64,
    pub monotonic_timestamp_ns: u64,
    pub captured_at_ms: i64,
    pub window_handle: u64,
    pub monitor_id: String,
    /// Supplied by the Windows helper. Android may omit this: the importer
    /// computes the immutable original hash, then the semantic ticket exposes it.
    pub expected_source_asset_id: Option<String>,
}
pub(crate) struct ValidatedCapture {
    capture: v1::CaptureInfo,
    expected: Option<AssetId>,
    width: u32,
    height: u32,
}
impl ValidatedCapture {
    pub(crate) fn new(value: CaptureImportDescriptor) -> Result<Self> {
        if !value.explicit_owner_action
            || !matches!(value.platform.as_str(), "windows" | "android")
            || !matches!(value.source_kind.as_str(), "window" | "monitor")
            || value.width == 0
            || value.height == 0
            || value.width > 32768
            || value.height > 32768
            || u64::from(value.width) * u64::from(value.height) > 50_000_000
            || value.monotonic_timestamp_ns == 0
            || value.monotonic_timestamp_ns > i64::MAX as u64
            || value.captured_at_ms < 0
            || value.frame_id == 0
            || value.geometry_revision == 0
            || value.monitor_id.len() > 256
            || value.monitor_id.contains('\0')
            || !value.dpi_scale.is_finite()
            || value.dpi_scale <= 0.0
            || value.dpi_scale > 10.0
            || i64::from(value.physical_x) + i64::from(value.width) > i64::from(i32::MAX)
            || i64::from(value.physical_y) + i64::from(value.height) > i64::from(i32::MAX)
            || (value.platform == "android"
                && (value.window_handle != 0 || value.physical_x != 0 || value.physical_y != 0))
        {
            return Err(CoreError::Invalid);
        }
        let session = Id::try_from(value.capture_session_id)?;
        let expected = value
            .expected_source_asset_id
            .map(AssetId::try_from)
            .transpose()?;
        let capture = v1::CaptureInfo {
            capture_session_id: Some(session.to_proto()),
            frame_id: value.frame_id,
            geometry: Some(v1::CaptureGeometry {
                source_kind: value.source_kind,
                window_handle: value.window_handle,
                monitor_id: value.monitor_id,
                client_rect_host: Some(v1::RectI {
                    x: value.physical_x,
                    y: value.physical_y,
                    w: value.width as i32,
                    h: value.height as i32,
                }),
                dpi_scale: value.dpi_scale,
                geometry_revision: value.geometry_revision,
                timestamp_ns: value.monotonic_timestamp_ns as i64,
            }),
            platform: value.platform,
            app_name: String::new(),
            window_title: String::new(),
            lossless: true,
            degraded: false,
            captured_at_ms: value.captured_at_ms,
        };
        vw_model::validate_capture(&capture)?;
        Ok(Self {
            capture,
            expected,
            width: value.width,
            height: value.height,
        })
    }
    pub(crate) fn verify_source(&self, source: &vw_raster::SourceInfo, bytes: &[u8]) -> Result<()> {
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || source.width != self.width
            || source.height != self.height
            || source.orientation_applied != 1
            || !matches!(source.bit_depth, 8 | 16)
            || self
                .expected
                .as_ref()
                .is_some_and(|expected| expected != &source.source_asset)
        {
            return Err(CoreError::Invalid);
        }
        Ok(())
    }
    pub(crate) fn info(&self) -> v1::CaptureInfo {
        self.capture.clone()
    }
}
#[uniffi::export]
pub async fn create_capture_project(
    options: CreateFileImageProject,
    capture: CaptureImportDescriptor,
    cancellation: Arc<Cancellation>,
) -> Result<Arc<ProjectSession>> {
    let admitted = ValidatedCapture::new(capture)?;
    if options.now_ms < admitted.capture.captured_at_ms {
        return Err(CoreError::Invalid);
    }
    crate::streaming_ffi::create_file_project_inner(options, cancellation, Some(admitted)).await
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // Invalid fixture setup fails the test.
    use super::*;
    fn descriptor() -> CaptureImportDescriptor {
        CaptureImportDescriptor {
            explicit_owner_action: true,
            capture_session_id: "019f0000-0000-7000-8000-000000000001".into(),
            frame_id: 1,
            geometry_revision: 1,
            platform: "android".into(),
            source_kind: "monitor".into(),
            physical_x: 0,
            physical_y: 0,
            width: 2,
            height: 3,
            dpi_scale: 1.0,
            monotonic_timestamp_ns: 100,
            captured_at_ms: 10,
            window_handle: 0,
            monitor_id: String::new(),
            expected_source_asset_id: None,
        }
    }
    #[test]
    fn no_capture_without_action_or_exact_geometry() {
        let mut value = descriptor();
        value.explicit_owner_action = false;
        assert!(ValidatedCapture::new(value).is_err());
        let mut value = descriptor();
        value.physical_x = 1;
        assert!(ValidatedCapture::new(value).is_err());
        let mut value = descriptor();
        value.width = 50000;
        assert!(ValidatedCapture::new(value).is_err());
    }
    #[test]
    fn bound_provenance_is_not_synthesized_from_import_time() {
        let value = ValidatedCapture::new(descriptor()).unwrap();
        assert_eq!(value.info().captured_at_ms, 10);
        assert_eq!(value.info().geometry.unwrap().timestamp_ns, 100);
    }
}

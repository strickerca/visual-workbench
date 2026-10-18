//! Explicit local source selections. A window grant never becomes a monitor grant.
use crate::{Cancellation, Error, FrameIdentity, FrameReceipt, Limits, Rect, Result, WindowTarget};
use serde::{Deserialize, Serialize};
use vw_geom::{CaptureGeometry, GeometryRevision, HostRect, PixelPoint, QuarterTurn};

pub const MAX_PICKER_WINDOWS: usize = 128;
pub const MAX_PICKER_MONITORS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorTarget {
    pub monitor: u64,
    /// Windows display device name, retained locally; never interpreted as a path.
    pub device: String,
    pub physical: Rect,
    pub dpi: u32,
    /// Display configuration metadata. WGC already returns desktop-oriented pixels.
    pub rotation: u32,
    pub primary: bool,
    pub observed_ns: u64,
}
impl MonitorTarget {
    pub fn validate(&self) -> Result<()> {
        self.physical.validate()?;
        if self.monitor == 0 || self.device.is_empty() || self.device.len() > 128
            || self.device.contains('\0') || !(48..=960).contains(&self.dpi)
            || self.rotation > 3 || self.observed_ns == 0
        { return Err(Error::Invalid); }
        Ok(())
    }
    pub fn unchanged(&self, current: &Self) -> Result<()> {
        self.validate()?; current.validate()?;
        if self.monitor != current.monitor || self.device != current.device
            || self.physical != current.physical || self.dpi != current.dpi
            || self.rotation != current.rotation || self.primary != current.primary
            || current.observed_ns < self.observed_ns
        { return Err(Error::Stale); }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerWindow { pub title: String, pub target: WindowTarget }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerCatalog {
    pub windows: Vec<PickerWindow>, pub monitors: Vec<MonitorTarget>, pub truncated: bool,
}
impl PickerCatalog {
    pub fn validate(&self, owner: u32) -> Result<()> {
        if self.windows.len() > MAX_PICKER_WINDOWS || self.monitors.len() > MAX_PICKER_MONITORS {
            return Err(Error::Limit);
        }
        for (i, value) in self.windows.iter().enumerate() {
            value.target.validate(owner)?;
            if value.title.len() > 4096 || value.title.contains('\0')
                || self.windows[..i].iter().any(|other| other.target.window == value.target.window)
            { return Err(Error::Invalid); }
        }
        for (i, value) in self.monitors.iter().enumerate() {
            value.validate()?;
            if self.monitors[..i].iter().any(|other| other.monitor == value.monitor) {
                return Err(Error::Invalid);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PickerSource {
    Window { target: WindowTarget },
    Monitor { target: MonitorTarget },
    /// Exact physical subrectangle on one monitor; never silently crosses displays.
    Region { target: MonitorTarget, region: Rect },
}
impl PickerSource {
    pub fn validate(&self, owner: u32, limits: Limits) -> Result<()> {
        match self {
            Self::Window { target } => { target.validate(owner)?; limits.image(target.frame.width, target.frame.height)?; }
            Self::Monitor { target } | Self::Region { target, .. } => {
                target.validate()?; limits.image(target.physical.width, target.physical.height)?;
            }
        }
        let rect = self.rect(); rect.validate()?;
        if let Self::Region { target, region } = self { region.offset_inside(target.physical)?; }
        limits.image(rect.width, rect.height)?;
        Ok(())
    }
    pub fn rect(&self) -> Rect {
        match self { Self::Window { target } => target.client,
            Self::Monitor { target } => target.physical, Self::Region { region, .. } => *region }
    }
    pub fn dpi(&self) -> u32 {
        match self { Self::Window { target } => target.dpi,
            Self::Monitor { target } | Self::Region { target, .. } => target.dpi }
    }
    pub fn kind(&self) -> &'static str {
        match self { Self::Window { .. } => "window", Self::Monitor { .. } => "monitor", Self::Region { .. } => "region" }
    }
    pub fn observed_ns(&self) -> u64 {
        match self { Self::Window { target } => target.observed_ns,
            Self::Monitor { target } | Self::Region { target, .. } => target.observed_ns }
    }
    /// Pixels are in the desktop orientation: rotation is not applied twice.
    pub fn geometry(&self, revision: u32) -> Result<CaptureGeometry> {
        if revision == 0 { return Err(Error::Invalid); }
        let rect = self.rect();
        CaptureGeometry::new(HostRect::new(PixelPoint { x: rect.x, y: rect.y }, rect.width, rect.height)
            .map_err(|_| Error::Invalid)?, f64::from(self.dpi()) / 96.0,
            GeometryRevision(u64::from(revision)), QuarterTurn::Zero).map_err(|_| Error::Invalid)
    }
    /// Frame coordinates are half-open pixel edges. Reversed drags are allowed;
    /// fractional edges cover their full pixel extent, without hidden clipping.
    pub fn region_from_frame_edges(&self, edges: [f64; 4], revision: u32) -> Result<Rect> {
        let geometry = self.geometry(revision)?;
        let [width, height] = geometry.frame_extent();
        if edges.iter().any(|v| !v.is_finite()) { return Err(Error::Invalid); }
        let left = edges[0].min(edges[2]).floor(); let top = edges[1].min(edges[3]).floor();
        let right = edges[0].max(edges[2]).ceil(); let bottom = edges[1].max(edges[3]).ceil();
        if left < 0.0 || top < 0.0 || right > f64::from(width) || bottom > f64::from(height)
            || right <= left || bottom <= top { return Err(Error::Invalid); }
        let first = geometry.frame_to_host(PixelPoint { x: left as i32, y: top as i32 }, geometry.revision())
            .map_err(|_| Error::Invalid)?;
        Rect { x: first.x, y: first.y, width: (right - left) as u32, height: (bottom - top) as u32 }.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerCaptureRequest {
    pub explicit_owner_action: bool, pub owner_process_id: u32, pub source: PickerSource,
    pub capture_session_id: String, pub frame_id: u64, pub geometry_revision: u32,
    pub output_directory: String, pub limits: Limits,
}
impl PickerCaptureRequest {
    pub fn validate(&self, cancel: &Cancellation) -> Result<()> {
        cancel.check()?;
        if !self.explicit_owner_action { return Err(Error::Consent); }
        if self.owner_process_id == 0 { return Err(Error::Invalid); }
        self.source.validate(self.owner_process_id, self.limits)?;
        vw_model::Id::try_from(self.capture_session_id.clone()).map_err(|_| Error::Invalid)?;
        if self.frame_id == 0 || self.frame_id > i64::MAX as u64 || self.geometry_revision == 0
            || self.output_directory.is_empty() || self.output_directory.len() > 4096
        { return Err(Error::Invalid); }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickerFrameReceipt {
    pub identity: FrameIdentity, pub source: PickerSource, pub source_asset_id: String,
    pub png_bytes: u64, pub width: u32, pub height: u32, pub bit_depth: u8,
    pub border_visible: bool, pub filename: String,
    /// Only window captures can collect that window's exact UIA subtree.
    pub window_receipt: Option<FrameReceipt>,
}
impl PickerFrameReceipt {
    pub fn validate(&self, request: &PickerCaptureRequest) -> Result<()> {
        self.identity.validate()?;
        vw_model::AssetId::try_from(self.source_asset_id.clone()).map_err(|_| Error::Invalid)?;
        let rect = request.source.rect();
        if self.source != request.source || self.identity.capture_session_id != request.capture_session_id
            || self.identity.frame_id != request.frame_id || self.identity.geometry_revision != request.geometry_revision
            || self.identity.source_kind != request.source.kind() || self.identity.client_rect != rect
            || self.identity.dpi_scale != f64::from(request.source.dpi()) / 96.0
            || self.identity.monotonic_timestamp_ns < request.source.observed_ns()
            || self.identity.platform != "windows" || self.width != rect.width || self.height != rect.height
            || self.bit_depth != 8 || !self.border_visible || self.png_bytes == 0
            || self.png_bytes > request.limits.png_bytes || self.filename != "capture.png"
        { return Err(Error::Invalid); }
        match (&self.source, &self.window_receipt) {
            (PickerSource::Window { target }, Some(frame)) => {
                if frame.target != *target || frame.identity != self.identity || !frame.lossless
                    || frame.width != self.width || frame.height != self.height || frame.bit_depth != 8
                    || frame.source_asset_id != self.source_asset_id || frame.png_bytes != self.png_bytes
                    || frame.filename != self.filename || !frame.border_visible
                    || self.identity.window_handle != target.window || !self.identity.monitor_id.is_empty()
                { return Err(Error::Invalid); }
            }
            (PickerSource::Monitor { target } | PickerSource::Region { target, .. }, None) => {
                if self.identity.window_handle != 0 || self.identity.monitor_id != target.device { return Err(Error::Invalid); }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
}

use crate::{Cancellation, Result, StreamError, rgba_bytes};
use vw_geom::{CaptureGeometry, GeometryRevision, HostRect, PixelPoint, QuarterTurn};
use vw_model::Id;
use vw_proto::v1;

/// All bytes are desktop-oriented physical pixels. DPI is metadata, never a
/// second scale. Geometry/source identity is constant within a stream epoch.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamGeometry {
    wire: v1::CaptureGeometry,
    width: u32,
    height: u32,
}
impl StreamGeometry {
    pub fn new(wire: &v1::CaptureGeometry, width: u32, height: u32) -> Result<Self> {
        rgba_bytes(width, height)?;
        let rect = wire.client_rect_host.as_ref().ok_or(StreamError::Invalid)?;
        if !matches!(wire.source_kind.as_str(), "window" | "monitor" | "region")
            || wire.monitor_id.len() > 128
            || wire.monitor_id.chars().any(char::is_control)
            || (wire.source_kind == "window" && (wire.window_handle == 0 || !wire.monitor_id.is_empty()))
            || (wire.source_kind == "monitor" && (wire.window_handle != 0 || wire.monitor_id.is_empty()))
            || (wire.source_kind == "region" && ((wire.window_handle == 0) == wire.monitor_id.is_empty()))
            || rect.w != width as i32 || rect.h != height as i32
            || wire.geometry_revision == 0 || wire.timestamp_ns <= 0
            || !wire.dpi_scale.is_finite() || wire.dpi_scale <= 0.0
        { return Err(StreamError::Invalid); }
        let host = HostRect::new(PixelPoint { x: rect.x, y: rect.y }, width, height).map_err(|_| StreamError::Invalid)?;
        CaptureGeometry::new(host, wire.dpi_scale, GeometryRevision(u64::from(wire.geometry_revision)), QuarterTurn::Zero)
            .map_err(|_| StreamError::Invalid)?;
        Ok(Self { wire: wire.clone(), width, height })
    }
    pub fn width(&self) -> u32 { self.width }
    pub fn height(&self) -> u32 { self.height }
    pub fn revision(&self) -> u32 { self.wire.geometry_revision }
    pub fn wire(&self) -> &v1::CaptureGeometry { &self.wire }
    /// Timestamp changes per captured frame; all other source fields must match.
    pub fn same_source(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height
            && self.wire.source_kind == other.wire.source_kind
            && self.wire.window_handle == other.wire.window_handle
            && self.wire.monitor_id == other.wire.monitor_id
            && self.wire.client_rect_host == other.wire.client_rect_host
            && self.wire.dpi_scale == other.wire.dpi_scale
            && self.wire.geometry_revision == other.wire.geometry_revision
    }
    pub fn mapping(&self) -> Result<CaptureGeometry> {
        let r = self.wire.client_rect_host.as_ref().ok_or(StreamError::Invalid)?;
        CaptureGeometry::new(HostRect::new(PixelPoint {x:r.x,y:r.y},self.width,self.height).map_err(|_|StreamError::Invalid)?,
            self.wire.dpi_scale,GeometryRevision(u64::from(self.revision())),QuarterTurn::Zero).map_err(|_|StreamError::Invalid)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameIdentity {
    pub session: Id,
    pub frame_id: u64,
    pub geometry: StreamGeometry,
    pub timestamp_ns: i64,
}
impl FrameIdentity {
    pub fn validate(&self) -> Result<()> {
        if self.frame_id == 0 || self.timestamp_ns <= 0 || self.timestamp_ns != self.geometry.wire.timestamp_ns {
            return Err(StreamError::Invalid);
        }
        Ok(())
    }
}

pub(crate) fn opaque(rgba: &[u8], width: u32, height: u32, cancel: &Cancellation) -> Result<()> {
    if rgba.len() != rgba_bytes(width,height)? { return Err(StreamError::Invalid); }
    for row in rgba.chunks_exact(width as usize * 4) {
        cancel.check()?;
        if row.chunks_exact(4).any(|p| p[3] != 255) { return Err(StreamError::Invalid); }
    }
    Ok(())
}

//! Bounded remote-edit identities, access-unit admission and helper IPC.
//! Transport acceptance and successful Windows injection are distinct.
pub mod annexb;
pub mod applied;
pub mod profile;
pub mod wire;
use serde::{Deserialize, Serialize};
pub const CAPABILITY: &str = "remote_edit_v1";
pub const MAX_ACCESS_UNIT: usize = 6 * 1024 * 1024; // leaves room in existing 8 MiB MEDIA quota
pub const MAX_CONFIG: usize = 8 * 1024; // fits bounded JSON IPC and CONTROL envelopes
pub const MAX_SIDE: u32 = 4096;
pub const BITRATE: u32 = 24_000_000;
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
pub enum Error {
    #[error("invalid remote-edit identity or bytes")]
    Invalid,
    #[error("bounded remote-edit capacity exhausted")]
    Limit,
    #[error("remote-edit target changed; explicit new grant required")]
    TargetChanged,
    #[error("remote-edit control is not granted")]
    Ungranted,
    #[error("remote-edit capability unavailable")]
    Unavailable,
    #[error("owned remote-edit operation timed out")]
    Timeout,
    #[error("owned remote-edit operation cancelled")]
    Cancelled,
    #[error("remote-edit owner sealed after partial input; held state unknown")]
    PartialInput,
    #[error("remote-edit platform operation failed at {phase} ({code:#x})")]
    Platform { phase: String, code: u32 },
    #[error("remote-edit IO failed")]
    Io,
    #[error("remote-edit retirement pending; owner sealed and retained")]
    RetirementPending,
}
pub type Result<T> = std::result::Result<T, Error>;
impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
    pub fn validate(self) -> Result<()> {
        if self.width == 0
            || self.height == 0
            || self.width > 32768
            || self.height > 32768
            || self.x.checked_add(self.width as i32).is_none()
            || self.y.checked_add(self.height as i32).is_none()
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn inside(self, parent: Self) -> bool {
        self.validate().is_ok()
            && parent.validate().is_ok()
            && self.x >= parent.x
            && self.y >= parent.y
            && i64::from(self.x) + i64::from(self.width)
                <= i64::from(parent.x) + i64::from(parent.width)
            && i64::from(self.y) + i64::from(self.height)
                <= i64::from(parent.y) + i64::from(parent.height)
    }
    pub fn contains(self, x: i32, y: i32) -> bool {
        self.validate().is_ok()
            && x >= self.x
            && y >= self.y
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub token: String,
    pub window: u64,
    pub process_id: u32,
    pub thread_id: u32,
    pub process_created: u64,
    pub window_rect: Rect,
    pub frame_rect: Rect,
    pub client_rect: Rect,
    pub dpi: u32,
    pub integrity: u32,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        id(&self.token)?;
        self.window_rect.validate()?;
        self.frame_rect.validate()?;
        self.client_rect.validate()?;
        if self.window == 0
            || self.process_id == 0
            || self.thread_id == 0
            || self.process_created == 0
            || !(48..=768).contains(&self.dpi)
            || !self.client_rect.inside(self.frame_rect)
            || !self.client_rect.inside(self.window_rect)
            || self.client_rect.width > MAX_SIDE
            || self.client_rect.height > MAX_SIDE
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn coded_size(&self) -> Result<(u32, u32)> {
        self.validate()?;
        Ok((
            (self.client_rect.width + 1) & !1,
            (self.client_rect.height + 1) & !1,
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub connection_epoch: u64,
    pub capture_session_id: String,
    pub source_generation: u64,
    pub target_token: String,
    pub geometry_revision: u32,
}
impl Scope {
    pub fn validate(&self) -> Result<()> {
        id(&self.capture_session_id)?;
        id(&self.target_token)?;
        if self.connection_epoch & 1 != 1
            || self.source_generation == 0
            || self.geometry_revision == 0
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Binding {
    pub scope: Scope,
    pub input_session_id: String,
}
impl Binding {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        id(&self.input_session_id)
    }
}
pub fn id(value: &str) -> Result<()> {
    vw_model::Id::try_from(value.to_owned())
        .map(|_| ())
        .map_err(|_| Error::Invalid)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoConfig {
    pub scope: Scope,
    pub generation: u64,
    pub visible_width: u32,
    pub visible_height: u32,
    pub coded_width: u32,
    pub coded_height: u32,
    pub vps: Vec<u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
    pub encoder: EncoderCapabilities,
}
impl VideoConfig {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.generation == 0
            || self.visible_width == 0
            || self.visible_height == 0
            || self.visible_width > MAX_SIDE
            || self.visible_height > MAX_SIDE
            || self.coded_width != ((self.visible_width + 1) & !1)
            || self.coded_height != ((self.visible_height + 1) & !1)
            || self.vps.len() + self.sps.len() + self.pps.len() > MAX_CONFIG
        {
            return Err(Error::Invalid);
        }
        annexb::config(&self.vps, 32)?;
        annexb::config(&self.sps, 33)?;
        annexb::config(&self.pps, 34)?;
        annexb::sps(&self.sps, self.coded_width, self.coded_height)?;
        if !self.encoder.hardware_enumerated
            || self.encoder.adapter_vendor != 0x8086
            || !self.encoder.intel_vendor_confirmed
            || !self.encoder.low_latency_control_accepted
            || !self.encoder.zero_b_control_accepted
            || !self.encoder.cbr_control_accepted
            || !self.encoder.maximum_bitrate_control_accepted
            || self.encoder.bitrate != BITRATE
            || self.encoder.one_frame_in_flight != 1
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderCapabilities {
    pub name: String,
    pub vendor_attribute: Option<String>,
    pub adapter_vendor: u32,
    pub hardware_enumerated: bool,
    pub intel_vendor_confirmed: bool,
    pub low_latency_control_accepted: bool,
    pub zero_b_control_accepted: bool,
    pub cbr_control_accepted: bool,
    pub maximum_bitrate_control_accepted: bool,
    pub bitrate: u32,
    pub one_frame_in_flight: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub scope: Scope,
    pub config_generation: u64,
    pub frame_id: u64,
    pub pts_100ns: i64,
    pub captured_qpc_100ns: u64,
    pub input_session_id: Option<String>,
    pub last_input_seq_applied: u64,
    pub keyframe: bool,
    pub annexb: Vec<u8>,
}
impl Frame {
    pub fn validate(&self, config: &VideoConfig) -> Result<()> {
        config.validate()?;
        if self.scope != config.scope
            || self.config_generation != config.generation
            || self.frame_id == 0
            || self.pts_100ns <= 0
            || self.captured_qpc_100ns == 0
            || self.annexb.is_empty()
            || self.annexb.len() > MAX_ACCESS_UNIT
        {
            return Err(Error::Invalid);
        }
        if let Some(id_value) = &self.input_session_id {
            id(id_value)?;
        } else if self.last_input_seq_applied != 0 {
            return Err(Error::Invalid);
        }
        let unit = annexb::access_unit(&self.annexb)?;
        if unit.idr != self.keyframe
            || unit.vps.is_some_and(|v| v != config.vps)
            || unit.sps.is_some_and(|v| v != config.sps)
            || unit.pps.is_some_and(|v| v != config.pps)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

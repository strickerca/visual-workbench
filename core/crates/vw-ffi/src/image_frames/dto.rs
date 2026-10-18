use super::*;

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct ImageFrameSettings {
    pub crop_x: u32,
    pub crop_y: u32,
    pub crop_width: u32,
    pub crop_height: u32,
    pub clockwise_degrees: u16,
    pub output_width: u32,
    pub output_height: u32,
}
impl ImageFrameSettings {
    pub(super) fn validate(&self) -> WorkflowResult<()> {
        if self.crop_width == 0
            || self.crop_height == 0
            || !matches!(self.clockwise_degrees, 0 | 90 | 180 | 270)
            || !(1..=32_768).contains(&self.output_width)
            || !(1..=32_768).contains(&self.output_height)
        {
            return Err(WorkflowError::Invalid);
        }
        if u64::from(self.output_width) * u64::from(self.output_height) > 50_000_000 {
            return Err(WorkflowError::Limit);
        }
        Ok(())
    }
    pub(super) fn frame(&self, source: [u32; 2]) -> WorkflowResult<vw_geom::ImageOutputFrame> {
        self.validate()?;
        Ok(vw_geom::ImageOutputFrame::new(
            source,
            [self.crop_x, self.crop_y, self.crop_width, self.crop_height],
            vw_geom::QuarterTurn::from_degrees(i32::from(self.clockwise_degrees))?,
            [self.output_width, self.output_height],
        )?)
    }
    pub(super) fn from_frame(frame: vw_geom::ImageOutputFrame) -> Self {
        let [crop_x, crop_y, crop_width, crop_height] = frame.crop();
        let [output_width, output_height] = frame.output_size();
        let clockwise_degrees = match frame.rotation() {
            vw_geom::QuarterTurn::Zero => 0,
            vw_geom::QuarterTurn::Clockwise90 => 90,
            vw_geom::QuarterTurn::Half => 180,
            vw_geom::QuarterTurn::Clockwise270 => 270,
        };
        Self {
            crop_x,
            crop_y,
            crop_width,
            crop_height,
            clockwise_degrees,
            output_width,
            output_height,
        }
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageFrameEntry {
    pub layer_id: String,
    pub object_id: String,
    pub enabled: bool,
    pub locked: bool,
    pub settings: ImageFrameSettings,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageFramesSnapshot {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub source_width: u32,
    pub source_height: u32,
    pub source_bit_depth: u8,
    pub frame: Option<ImageFrameEntry>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ImageFrameCommand {
    Set {
        layer_id: String,
        object_id: String,
        settings: ImageFrameSettings,
        enabled: bool,
    },
    Remove {
        layer_id: String,
        object_id: String,
    },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageFramePreview {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub frame: Option<ImageFrameSettings>,
    pub output_width: u32,
    pub output_height: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Explicit sRGB RGBA8 display conversion; never original/export bytes.
    pub rgba: Vec<u8>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageEditingPreview {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Canonical annotations and tones in D. The viewport must not double-paint
    /// canonical annotations over this explicit sRGB display derivative.
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImageEditingBackground {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub source_bit_depth: u8,
    pub width: u32,
    pub height: u32,
    /// Complete canonical D composition in display sRGB, including annotations.
    pub rgba: Vec<u8>,
    /// Actual display profile produced by the canonical display conversion.
    pub icc_profile: Vec<u8>,
}

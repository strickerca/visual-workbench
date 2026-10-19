use super::*;

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ImageToneSettings {
    pub brightness: f64,
    pub contrast: f64,
    pub input_black: f64,
    pub input_white: f64,
    pub gamma: f64,
    pub output_black: f64,
    pub output_white: f64,
}
impl ImageToneSettings {
    pub(super) fn wire(&self) -> WorkflowResult<pb::Adjustment> {
        if ![
            self.brightness,
            self.contrast,
            self.input_black,
            self.input_white,
            self.gamma,
            self.output_black,
            self.output_white,
        ]
        .iter()
        .all(|v| v.is_finite())
            || !(-1.0..=1.0).contains(&self.brightness)
            || !(-1.0..=1.0).contains(&self.contrast)
            || ![
                self.input_black,
                self.input_white,
                self.output_black,
                self.output_white,
            ]
            .iter()
            .all(|v| (0.0..=1.0).contains(v))
            || self.input_black >= self.input_white
            || self.output_black > self.output_white
            || self.gamma <= 0.0
        {
            return Err(WorkflowError::Invalid);
        }
        Ok(pb::Adjustment {
            brightness: self.brightness,
            contrast: self.contrast,
            levels: vec![
                self.input_black,
                self.input_white,
                self.gamma,
                self.output_black,
                self.output_white,
            ],
        })
    }
    pub(super) fn from_wire(value: &pb::Adjustment) -> WorkflowResult<Self> {
        if value.levels.len() != 5 {
            return Err(WorkflowError::Invalid);
        }
        let result = Self {
            brightness: value.brightness,
            contrast: value.contrast,
            input_black: value.levels[0],
            input_white: value.levels[1],
            gamma: value.levels[2],
            output_black: value.levels[3],
            output_white: value.levels[4],
        };
        result.wire()?;
        Ok(result)
    }
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ToneControlEntry {
    pub layer_id: String,
    pub object_id: String,
    pub enabled: bool,
    pub locked: bool,
    pub opacity: f64,
    pub settings: ImageToneSettings,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ToneControlSnapshot {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub source_width: u32,
    pub source_height: u32,
    pub source_bit_depth: u8,
    pub tones: Vec<ToneControlEntry>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ToneControlCommand {
    Set {
        layer_id: String,
        object_id: String,
        settings: ImageToneSettings,
        enabled: bool,
        opacity: f64,
    },
    Remove {
        layer_id: String,
        object_id: String,
    },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ToneControlPreview {
    pub binding: WorkflowBinding,
    pub source_asset_id: String,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Explicit sRGB RGBA8 display conversion of one exact source-space tile.
    pub rgba: Vec<u8>,
}

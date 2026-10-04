use crate::{Point, ProjectInfo, QueryRect};

/// Exact visible (including optimistic pending edits) document binding. A host
/// sequence alone does not identify the client's current visible state.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SelectionBinding {
    pub project_id: String,
    pub document_id: String,
    pub source_asset_id: String,
    pub host_seq: u64,
    pub state_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SelectionVersion {
    pub object_id: String,
    pub asset_id: String,
    /// Canonical MaskOp history length, not the math library's local counter.
    pub version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct SelectionRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionDocument {
    pub binding: SelectionBinding,
    pub width: u32,
    pub height: u32,
    pub source_bit_depth: u32,
    pub revision: ProjectInfo,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionSnapshot {
    pub binding: SelectionBinding,
    pub selection: SelectionVersion,
    pub layer_id: String,
    pub width: u32,
    pub height: u32,
    pub nonzero_bounds: Option<SelectionRegion>,
    pub editable: bool,
    pub visible: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionTile {
    pub region: SelectionRegion,
    /// Row-major 8-bit coverage, never display-color-converted. At most 256².
    pub coverage: Vec<u8>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionTiles {
    pub binding: SelectionBinding,
    pub selection: SelectionVersion,
    pub tiles: Vec<SelectionTile>,
}

#[derive(Clone, Copy, Debug, uniffi::Enum)]
pub enum SelectionCombine {
    Add,
    Subtract,
    Intersect,
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum SelectionTarget {
    /// A missing layer is created atomically as a mask layer. Existing layers
    /// must be unlocked mask layers in this same document.
    New {
        object_id: String,
        layer_id: String,
    },
    Existing {
        selection: SelectionVersion,
    },
}

#[derive(Clone, Debug, uniffi::Enum)]
pub enum SelectionOperation {
    Rectangle {
        rectangle: QueryRect,
        combine: SelectionCombine,
    },
    Lasso {
        points: Vec<Point>,
        combine: SelectionCombine,
    },
    /// Subtract + paint is the nondestructive mask eraser. Radius is in D pixels.
    Paint {
        points: Vec<Point>,
        radius: f64,
        opacity: u8,
        combine: SelectionCombine,
    },
    Combine {
        other: SelectionVersion,
        combine: SelectionCombine,
    },
    Invert,
    Expand {
        radius: u32,
    },
    Shrink {
        radius: u32,
    },
    Feather {
        radius: u32,
    },
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionEdit {
    pub binding: SelectionBinding,
    pub transaction_id: String,
    pub device_id: String,
    pub lamport: u64,
    pub created_at_ms: i64,
    pub target: SelectionTarget,
    pub operation: SelectionOperation,
    pub memory_budget_bytes: u64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionReceipt {
    pub transaction_id: String,
    pub snapshot: SelectionSnapshot,
    pub revision: ProjectInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SelectionExportKind {
    Mask,
    Cutout,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionExportOptions {
    pub binding: SelectionBinding,
    pub selection: SelectionVersion,
    pub kind: SelectionExportKind,
    /// None means the whole document; a crop is explicit and never resampled.
    pub region: Option<SelectionRegion>,
    /// Existing private directory and a new direct child; never a user file to
    /// overwrite. The app publishes the completed result through its picker.
    pub work_directory: String,
    pub output_path: String,
    pub memory_budget_bytes: u64,
    pub max_encoded_bytes: u64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SelectionExportReceipt {
    pub binding: SelectionBinding,
    pub selection: SelectionVersion,
    pub kind: SelectionExportKind,
    pub region: SelectionRegion,
    pub output_bit_depth: u8,
    pub encoded_bytes: u64,
    pub blake3: String,
    /// Selection/source/region binding for the app's export record. Cutout PNG
    /// also embeds the raster revision/source record; mask PNG embeds this one.
    pub metadata_json: String,
    pub revision: ProjectInfo,
    /// Conservative incremental operation reservation, not measured process RSS.
    pub estimated_peak_bytes: u64,
}

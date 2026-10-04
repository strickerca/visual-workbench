#[derive(Clone, Debug, uniffi::Record)]
pub struct CreateImageProject {
    pub path: String,
    pub project_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub device_id: String,
    pub title: String,
    pub source: Vec<u8>,
    pub now_ms: i64,
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ProjectInfo {
    pub project_id: String,
    pub title: String,
    pub device_id: String,
    pub next_lamport: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub host_seq: u64,
    pub state_hash: String,
    pub document_ids: Vec<String>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct EditPrecondition {
    pub host_seq: u64,
    pub state_hash: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct StrokeOptions {
    pub gesture_id: String,
    pub transaction_id: String,
    pub object_id: String,
    pub document_id: String,
    pub layer_id: String,
    pub device_id: String,
    pub lamport: u64,
    pub created_at_ms: i64,
    pub family: String,
    pub width: f64,
    pub rgba: u32,
    pub stabilization: f32,
    /// Empty selects the brush default; otherwise exactly eight monotonic values.
    pub pressure_curve: Vec<f64>,
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SampleBatch {
    pub sequence: u64,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub t_ms: Vec<u32>,
    pub pressure: Vec<f32>,
    pub tilt: Vec<f32>,
    pub orientation: Vec<f32>,
}
#[derive(Clone, Debug, Default, PartialEq, uniffi::Record)]
pub struct Contours {
    pub x: Vec<i64>,
    pub y: Vec<i64>,
    pub ends: Vec<u32>,
}
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct InkUpdate {
    pub sequence: u64,
    pub first_polygon: u64,
    pub sample_count: u64,
    pub contours: Contours,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct QueryRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RenderItem {
    pub object_id: String,
    pub layer_id: String,
    pub layer_opacity: f64,
    pub layer_blend: String,
    pub bounds: QueryRect,
    pub object_protobuf: Vec<u8>,
    pub stroke_contours: Contours,
    pub transform: Transform,
    pub style: ObjectStyle,
    pub shape: DrawShape,
    pub locked: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct RenderList {
    pub revision: ProjectInfo,
    pub items: Vec<RenderItem>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ImageFormat {
    Png8,
    Png16,
    Jpeg { quality: u8 },
    WebpLossless,
    WebpLossy { quality: u8 },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ExportOptions {
    pub document_id: String,
    pub format: ImageFormat,
    pub marked: bool,
    pub region: Option<QueryRect>,
    pub matte_rgb: Option<u32>,
    pub convert_to_srgb: bool,
    pub assume_untagged_srgb: bool,
    pub allow_depth_reduction: bool,
    pub memory_budget_bytes: u64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ExportResult {
    pub bytes: Vec<u8>,
    pub blake3: String,
    pub metadata_json: String,
    pub revision: ProjectInfo,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum Outline {
    Move {
        x: f32,
        y: f32,
    },
    Line {
        x: f32,
        y: f32,
    },
    Quad {
        x1: f32,
        y1: f32,
        x: f32,
        y: f32,
    },
    Cubic {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        x: f32,
        y: f32,
    },
    Close,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct GlyphOutline {
    pub glyph_id: u32,
    pub cluster: u64,
    pub font: String,
    pub x: f32,
    pub y: f32,
    pub advance: f32,
    pub outline: Vec<Outline>,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct TextLayout {
    pub algorithm_version: u32,
    pub glyphs: Vec<GlyphOutline>,
    pub width: f32,
    pub height: f32,
    pub line_height: f32,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum ChangeKind {
    Opened,
    Committed,
    ExportStarted,
    ExportReady,
    ExportFailed,
    Closed,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct StateChange {
    pub sequence: u64,
    pub kind: ChangeKind,
    pub project: ProjectInfo,
}

#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Transform {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct ObjectStyle {
    pub rgba: u32,
    pub width: f64,
    pub screen_constant_width: bool,
    pub fill: Option<u32>,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum DrawShape {
    Stroke {
        family: String,
    },
    Line {
        points: Vec<Point>,
    },
    Arrow {
        points: Vec<Point>,
    },
    Rectangle {
        rectangle: QueryRect,
    },
    Ellipse {
        rectangle: QueryRect,
    },
    Polygon {
        points: Vec<Point>,
        closed: bool,
    },
    Text {
        anchor: Point,
        text: String,
        font: String,
        size: f64,
        outline: Vec<Outline>,
    },
    Marker {
        number: u32,
        point: Point,
        rectangle: Option<QueryRect>,
    },
    Guide {
        rectangle: QueryRect,
    },
    Result {
        asset_id: String,
        result_id: String,
    },
    Adjustment,
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum NewShape {
    Line {
        points: Vec<Point>,
    },
    Arrow {
        points: Vec<Point>,
    },
    Rectangle {
        rectangle: QueryRect,
    },
    Ellipse {
        rectangle: QueryRect,
    },
    Text {
        anchor: Point,
        text: String,
        font: String,
        size: f64,
    },
}
#[derive(Clone, Debug, uniffi::Enum)]
pub enum EditCommand {
    Create {
        object_id: String,
        layer_id: String,
        shape: NewShape,
        style: ObjectStyle,
        transform: Transform,
    },
    Delete {
        object_id: String,
    },
    Transform {
        object_id: String,
        transform: Transform,
    },
    SetText {
        object_id: String,
        text: String,
        font: String,
        size: f64,
    },
    SetStyle {
        object_id: String,
        style: ObjectStyle,
    },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct EditOptions {
    pub transaction_id: String,
    pub document_id: String,
    pub device_id: String,
    pub lamport: u64,
    pub created_at_ms: i64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct LayerInfo {
    pub id: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f64,
    pub blend: String,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct DocumentSnapshot {
    pub document_id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub layers: Vec<LayerInfo>,
    pub render: RenderList,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct BackgroundImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub source_asset_id: String,
    pub source_bit_depth: u8,
    pub icc_profile: Vec<u8>,
}
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct Camera {
    pub center: Point,
    pub scale: f64,
    pub rotation: f64,
    pub viewport_width: f64,
    pub viewport_height: f64,
}

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
    /// PDF page selected by the caller; checked atomically against every target.
    pub page_index: Option<u32>,
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
    Bmp8,
    Tiff8,
    Tiff16,
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
    pub capture_provenance_json: Option<String>,
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
    /// The ordinary editing canvas shows the original; bounded tone preview
    /// and marked export evaluate these recognized canonical parameters.
    pub has_tone_adjustments: bool,
}
/// A stroke the caller still holds from an earlier snapshot of the document.
#[derive(Clone, Debug, uniffi::Record)]
pub struct KnownStroke {
    pub object_id: String,
    /// The 32-byte digest returned with that stroke.
    pub digest: Vec<u8>,
    /// The `cost` returned with that stroke.
    pub cost: u64,
}
/// One place in the display list.
#[derive(Clone, Debug, uniffi::Record)]
pub struct RenderItemChange {
    pub object_id: String,
    /// Strokes only: pass it back as `KnownStroke::digest` next time.
    pub digest: Option<Vec<u8>>,
    /// What this entry counts toward the size limit of a complete read of the
    /// document, whether or not the item is sent this time. A kept stroke
    /// still fills the caller's memory, so the limit is the same one
    /// `document_snapshot` applies. Pass it back as `KnownStroke::cost`.
    pub cost: u64,
    /// Absent when the stroke the caller holds under this digest is still
    /// exact; its layer fields below may nevertheless have changed.
    pub item: Option<RenderItem>,
    pub layer_opacity: f64,
    pub layer_blend: String,
    pub locked: bool,
}
/// `DocumentSnapshot` with its display list given as changes.
#[derive(Clone, Debug, uniffi::Record)]
pub struct DocumentSnapshotChanges {
    pub document_id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub layers: Vec<LayerInfo>,
    pub revision: ProjectInfo,
    pub items: Vec<RenderItemChange>,
    pub has_tone_adjustments: bool,
}
/// `RenderItem` with its stroke outline packed into one byte buffer, for the
/// complete document reads the apps repeat after every save. It leaves out
/// the stored object bytes (`RenderItem::object_protobuf`), which no app reads.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackedRenderItem {
    pub object_id: String,
    pub layer_id: String,
    pub layer_opacity: f64,
    pub layer_blend: String,
    pub bounds: QueryRect,
    /// Exactly the vertices and contour ends of `RenderItem::stroke_contours`.
    /// Empty when the item has no contour. Otherwise a sequence of unsigned
    /// LEB128 numbers (seven bits per byte, lowest group first, the high bit
    /// set on every byte of a number but its last): the format version 1, the
    /// contour count, the vertex count, then for each contour its vertex count
    /// followed by each vertex as two signed numbers dx and dy. A signed number
    /// n is sent as 2n when n >= 0 and as -2n - 1 otherwise (zigzag). dx and dy
    /// are the vertex minus the previous vertex of the item (the first minus
    /// 0, 0) in the same 1/256 document units, in wrapping 64-bit arithmetic,
    /// so every coordinate comes back as the same 64-bit value.
    pub contours: Vec<u8>,
    pub transform: Transform,
    pub style: ObjectStyle,
    pub shape: DrawShape,
    pub locked: bool,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackedRenderList {
    pub revision: ProjectInfo,
    pub items: Vec<PackedRenderItem>,
}
/// `DocumentSnapshot` with its display items as `PackedRenderItem`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackedDocumentSnapshot {
    pub document_id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub layers: Vec<LayerInfo>,
    pub render: PackedRenderList,
    pub has_tone_adjustments: bool,
}
/// `RenderItemChange` with a `PackedRenderItem`. Its `cost` is what the packed
/// item counts; pass it back only to `document_snapshot_changes_packed`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackedRenderItemChange {
    pub object_id: String,
    pub digest: Option<Vec<u8>>,
    pub cost: u64,
    pub item: Option<PackedRenderItem>,
    pub layer_opacity: f64,
    pub layer_blend: String,
    pub locked: bool,
}
/// `DocumentSnapshotChanges` with its entries as `PackedRenderItemChange`.
#[derive(Clone, Debug, uniffi::Record)]
pub struct PackedDocumentSnapshotChanges {
    pub document_id: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u32,
    pub layers: Vec<LayerInfo>,
    pub revision: ProjectInfo,
    pub items: Vec<PackedRenderItemChange>,
    pub has_tone_adjustments: bool,
}
/// What `StrokeGesture::commit_added` returns.
#[derive(Clone, Debug, uniffi::Record)]
pub struct StrokeSave {
    /// The project after the save: what `StrokeGesture::commit` returns.
    pub revision: ProjectInfo,
    /// Set when all the save did to the display list of its document was to
    /// add this stroke to it. Unset when it did more or that cannot be shown:
    /// the project is a mirror, which queues its edit; the stroke made a new
    /// highlighter layer; its layer is not drawn; the document is read page
    /// by page; or this is a retry of a finished save. Read the document then.
    pub added: Option<AddedStroke>,
}
/// A saved stroke as the display list of its document holds it.
#[derive(Clone, Debug, uniffi::Record)]
pub struct AddedStroke {
    /// The project just before the save. Only a caller that holds the document
    /// at exactly this revision may add `item` to what it holds; it then
    /// holds the document at `StrokeSave::revision`.
    pub previous: ProjectInfo,
    /// The stroke as `document_snapshot_changes_packed` sends it. A read lists
    /// it after every other item of its layer, and the layers in the order of
    /// `PackedDocumentSnapshotChanges::layers`.
    pub item: PackedRenderItem,
    /// The digest and cost `document_snapshot_changes_packed` gives for it:
    /// pass them back as its `KnownStroke`.
    pub digest: Vec<u8>,
    pub cost: u64,
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

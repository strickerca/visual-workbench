use crate::{
    Cancellation, CaptureContext, Error, MAX_DEPTH, MAX_ELEMENTS, MAX_TEXT_BYTES,
    MAX_TEXT_CHARACTERS, Result, check, codec, text,
};
use serde::{
    Deserialize, Serialize,
    de::{SeqAccess, Visitor},
};
use std::collections::BTreeMap;
use vw_model::Id;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Uia,
    AndroidAx,
    ChromiumUia,
}
impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Uia => "uia",
            Self::AndroidAx => "android_ax",
            Self::ChromiumUia => "chromium_uia",
        }
    }
}
/// Android adapters must map display rotation into captured screenshot pixels
/// before selecting CapturePixels. Windows H coordinates are already physical.
#[derive(Clone, Copy)]
pub enum BoundsSpace {
    HostPhysical,
    CapturePixels,
}

/// Platform-owned bounded collection input. Text longer than 200 Unicode scalars
/// is refused, never silently converted into a different instruction.
pub struct CapturedElement {
    pub local_id: String,
    pub parent_local_id: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    /// Continuous [x,y,width,height] edges in the declared physical space.
    pub bounds: [f64; 4],
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Element {
    pub eid: String,
    pub parent: Option<String>,
    pub name: String,
    pub role: String,
    pub automation_id: Option<String>,
    pub resource_id: Option<String>,
    pub html_id: Option<String>,
    pub bounds_document: [f64; 4],
    pub bounds_clipped: bool,
    pub text: String,
    pub enabled: bool,
    pub focused: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Data {
    pub schema_version: u32,
    pub project_id: Id,
    pub snapshot_id: Id,
    pub document_id: Id,
    pub capture: CaptureContext,
    pub platform: Platform,
    pub frame_delta_ms: i32,
    /// Caller-observed collection elapsed time, not a claimed platform guarantee.
    pub collection_elapsed_ms: u64,
    pub text_is_untrusted: bool,
    #[serde(deserialize_with = "elements")]
    pub elements: Vec<Element>,
}
/// Immutable validated data. Mutating an Element clone cannot alter this snapshot.
pub struct Snapshot {
    pub(crate) data: Data,
}
impl Snapshot {
    pub fn project_id(&self) -> &Id {
        &self.data.project_id
    }
    pub fn id(&self) -> &Id {
        &self.data.snapshot_id
    }
    pub fn document_id(&self) -> &Id {
        &self.data.document_id
    }
    pub fn context(&self) -> &CaptureContext {
        &self.data.capture
    }
    pub fn platform(&self) -> Platform {
        self.data.platform
    }
    pub fn elements(&self) -> &[Element] {
        &self.data.elements
    }
    pub fn frame_delta_ms(&self) -> i32 {
        self.data.frame_delta_ms
    }
    pub fn collection_elapsed_ms(&self) -> u64 {
        self.data.collection_elapsed_ms
    }
    pub fn uia_collection_within_target(&self) -> Option<bool> {
        (self.platform() != Platform::AndroidAx)
            .then_some(self.data.collection_elapsed_ms <= crate::UIA_CAPTURE_TARGET_MS)
    }
    /// Preserve an already validated capture while appending it to another
    /// project. This is identity rebinding, never a new OS collection: document,
    /// snapshot, element IDs, full capture geometry and literal text are exact.
    /// The caller must also copy that exact capture document and original.
    pub fn for_project(&self, project_id: Id, cancel: &dyn Cancellation) -> Result<Self> {
        check(cancel)?;
        codec::admission(&self.data, crate::MAX_JSON_BYTES)?;
        validate(&self.data, cancel)?;
        let data = Data {
            schema_version: self.data.schema_version,
            project_id,
            snapshot_id: self.data.snapshot_id.clone(),
            document_id: self.data.document_id.clone(),
            capture: self.data.capture.clone(),
            platform: self.data.platform,
            frame_delta_ms: self.data.frame_delta_ms,
            collection_elapsed_ms: self.data.collection_elapsed_ms,
            text_is_untrusted: self.data.text_is_untrusted,
            elements: self.data.elements.clone(),
        };
        check(cancel)?;
        Ok(Self { data })
    }

    pub fn encode(&self, cancel: &dyn Cancellation) -> Result<Vec<u8>> {
        codec::encode(&self.data, cancel)
    }
    pub(crate) fn decode(bytes: &[u8], cancel: &dyn Cancellation) -> Result<Self> {
        let json = codec::decode(bytes, cancel)?;
        let data: Data = serde_json::from_slice(&json).map_err(|_| Error::Json)?;
        validate(&data, cancel)?;
        codec::admission(&data, crate::MAX_JSON_BYTES)?;
        // The versioned envelope is canonical JSON, including nested generated
        // geometry records. Unknown nested fields and alternate encodings cannot
        // be silently accepted and then disappear on a later export.
        if serde_json::to_vec(&data).map_err(|_| Error::Json)? != json {
            return Err(Error::Json);
        }
        Ok(Self { data })
    }
}
#[allow(clippy::too_many_arguments)]
pub(crate) fn capture(
    project_id: Id,
    document_id: Id,
    snapshot_id: Id,
    context: CaptureContext,
    platform: Platform,
    frame_delta_ms: i32,
    collection_elapsed_ms: u64,
    space: BoundsSpace,
    raw: Vec<CapturedElement>,
    cancel: &dyn Cancellation,
) -> Result<Snapshot> {
    check(cancel)?;
    if raw.len() > MAX_ELEMENTS {
        return Err(Error::Limit("elements"));
    }
    let mut total = 0;
    // Admit all caller strings before namespacing, sorting or cloning any input.
    for element in &raw {
        check(cancel)?;
        total += text(&element.local_id, 256, true)?;
        if let Some(parent) = &element.parent_local_id {
            total += text(parent, 256, true)?;
        }
        total += strings(
            &element.name,
            &element.role,
            [
                &element.automation_id,
                &element.resource_id,
                &element.html_id,
            ],
            &element.text,
        )?;
        if total > MAX_TEXT_BYTES {
            return Err(Error::Limit("total text"));
        }
        rectangle(element.bounds)?;
    }
    let prefix = format!("{snapshot_id}/");
    let [width, height] = context.extent()?;
    let [ox, oy] = context.host_origin()?;
    let mut elements = Vec::with_capacity(raw.len());
    for element in raw {
        check(cancel)?;
        let mut bounds = element.bounds;
        if matches!(space, BoundsSpace::HostPhysical) {
            if platform == Platform::AndroidAx {
                return Err(Error::Invalid("Android host coordinates"));
            }
            bounds[0] -= ox;
            bounds[1] -= oy;
        }
        let left = bounds[0].clamp(0.0, width);
        let top = bounds[1].clamp(0.0, height);
        let right = (bounds[0] + bounds[2]).clamp(0.0, width);
        let bottom = (bounds[1] + bounds[3]).clamp(0.0, height);
        let clipped = [left, top, right - left, bottom - top];
        elements.push(Element {
            eid: format!("{prefix}{}", element.local_id),
            parent: element
                .parent_local_id
                .map(|value| format!("{prefix}{value}")),
            name: element.name,
            role: element.role,
            automation_id: element.automation_id,
            resource_id: element.resource_id,
            html_id: element.html_id,
            bounds_document: clipped,
            bounds_clipped: clipped != bounds,
            text: element.text,
            enabled: element.enabled,
            focused: element.focused,
        });
    }
    elements.sort_unstable_by(|a, b| a.eid.cmp(&b.eid));
    let data = Data {
        schema_version: 1,
        project_id,
        snapshot_id,
        document_id,
        capture: context,
        platform,
        frame_delta_ms,
        collection_elapsed_ms,
        text_is_untrusted: true,
        elements,
    };
    validate(&data, cancel)?;
    codec::admission(&data, crate::MAX_JSON_BYTES)?;
    Ok(Snapshot { data })
}
fn strings(name: &str, role: &str, ids: [&Option<String>; 3], captured: &str) -> Result<usize> {
    let mut total =
        text(name, 4096, false)? + text(role, 256, false)? + text(captured, 800, false)?;
    if captured.chars().count() > MAX_TEXT_CHARACTERS {
        return Err(Error::Limit("captured text characters"));
    }
    for id in ids.into_iter().flatten() {
        total += text(id, 1024, true)?;
    }
    Ok(total)
}
pub(crate) fn rectangle(value: [f64; 4]) -> Result<vw_geom::Rect> {
    if value
        .iter()
        .any(|v| !v.is_finite() || v.abs() > 1_000_000_000.0)
    {
        return Err(Error::Invalid("bounds"));
    }
    Ok(vw_geom::Rect::new(value[0], value[1], value[2], value[3])?)
}
fn validate(data: &Data, cancel: &dyn Cancellation) -> Result<()> {
    check(cancel)?;
    if data.schema_version != 1 || !data.text_is_untrusted {
        return Err(Error::Invalid("schema or trust"));
    }
    data.capture.validate(data.platform)?;
    if data.elements.len() > MAX_ELEMENTS {
        return Err(Error::Limit("elements"));
    }
    let [width, height] = data.capture.extent()?;
    let prefix = format!("{}/", data.snapshot_id);
    let mut total = 0;
    let mut by_id = BTreeMap::new();
    let mut previous: Option<&str> = None;
    for (index, element) in data.elements.iter().enumerate() {
        check(cancel)?;
        if previous.is_some_and(|p| p >= element.eid.as_str()) {
            return Err(Error::Invalid("element order or duplicate ID"));
        }
        previous = Some(&element.eid);
        let local = element
            .eid
            .strip_prefix(&prefix)
            .ok_or(Error::Invalid("element namespace"))?;
        total += text(local, 256, true)? + prefix.len();
        if let Some(parent) = &element.parent {
            total += text(
                parent
                    .strip_prefix(&prefix)
                    .ok_or(Error::Invalid("parent namespace"))?,
                256,
                true,
            )? + prefix.len();
        }
        total += strings(
            &element.name,
            &element.role,
            [
                &element.automation_id,
                &element.resource_id,
                &element.html_id,
            ],
            &element.text,
        )?;
        if total > MAX_TEXT_BYTES {
            return Err(Error::Limit("total text"));
        }
        let rect = rectangle(element.bounds_document)?;
        if rect.left() < 0.0 || rect.top() < 0.0 || rect.right() > width || rect.bottom() > height {
            return Err(Error::Invalid("document bounds"));
        }
        by_id.insert(element.eid.as_str(), index);
    }
    // Iterative bounded parent walks: <=4096*128 links, no recursive stack use.
    for element in &data.elements {
        check(cancel)?;
        let mut parent = element.parent.as_deref();
        let mut depth = 0;
        while let Some(eid) = parent {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(Error::Limit("parent depth"));
            }
            if eid == element.eid {
                return Err(Error::Invalid("parent cycle"));
            }
            let index = *by_id.get(eid).ok_or(Error::Invalid("missing parent"))?;
            parent = data.elements[index].parent.as_deref();
        }
    }
    Ok(())
}
fn elements<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> std::result::Result<Vec<Element>, D::Error> {
    struct BoundedElements;
    impl<'de> Visitor<'de> for BoundedElements {
        type Value = Vec<Element>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded elements")
        }
        fn visit_seq<A: SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(element) = seq.next_element()? {
                if result.len() == MAX_ELEMENTS {
                    return Err(serde::de::Error::custom("element limit"));
                }
                result.push(element);
            }
            Ok(result)
        }
    }
    decoder.deserialize_seq(BoundedElements)
}

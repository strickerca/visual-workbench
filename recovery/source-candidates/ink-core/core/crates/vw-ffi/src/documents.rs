//! Page-aware storage/application boundary. Arbitrary PDF/SVG parsing runs in
//! a separate isolated worker; these methods validate only numeric records.
use crate::{project::SessionPermit, worker::startup, *};
use std::{io::Read, path::Path, sync::Arc};
use vw_model::{AssetId, DeviceId, Document, Id, Layer, PdfPage, Project};
use vw_proto::v1 as pb;

mod jobs;
pub use jobs::*;
mod overlays;
pub use overlays::EditableOverlay;

pub(crate) const DOCUMENT_SOURCE_LIMIT: u64 = 64 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum EditableDocumentKind { Pdf, Svg }
#[derive(Clone, Debug, uniffi::Record)]
pub struct DocumentPageGeometry {
    pub page: u32, pub width: f64, pub height: f64,
    /// Native PDF crop box left/bottom/right/top, never display pixel bounds.
    pub crop: Vec<f64>, pub quarter_turns: u32, pub layer_id: String,
}
#[derive(Clone, uniffi::Record)]
pub struct CreateEditableDocument {
    pub path: String, pub project_id: String, pub document_id: String,
    pub device_id: String, pub title: String, pub source: Vec<u8>,
    pub source_blake3: String, pub kind: EditableDocumentKind,
    pub pages: Vec<DocumentPageGeometry>, pub now_ms: i64,
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct EditableDocumentDescriptor {
    pub document_id: String, pub source_asset_id: String,
    pub kind: EditableDocumentKind, pub pages: Vec<DocumentPageGeometry>,
    pub revision: ProjectInfo,
}
#[derive(Clone, uniffi::Record)]
pub struct EditableDocumentSource {
    pub descriptor: EditableDocumentDescriptor, pub bytes: Vec<u8>,
}
#[derive(Clone, uniffi::Record)]
pub struct PublishEditableCopy {
    pub document_id: String, pub source_asset_id: String, pub expected: ProjectInfo,
    pub destination: String, pub bytes: Vec<u8>, pub output_blake3: String,
}

fn dimensions(page: &DocumentPageGeometry) -> Result<(u32, u32)> {
    if ![page.width, page.height].into_iter().all(|n| n.is_finite() && n > 0.0 && n <= 1_000_000.0)
        || page.crop.len() != 4 || !page.crop.iter().all(|n| n.is_finite() && n.abs() <= 10_000_000.0)
        || page.quarter_turns > 3 { return Err(CoreError::Invalid); }
    Ok((page.width.ceil() as u32, page.height.ceil() as u32))
}
fn validate_pages(kind: EditableDocumentKind, pages: &[DocumentPageGeometry]) -> Result<()> {
    if pages.is_empty() || pages.len() > 1000 || (kind == EditableDocumentKind::Svg && pages.len() != 1) { return Err(CoreError::Invalid); }
    let mut layers = std::collections::BTreeSet::new();
    for (index, page) in pages.iter().enumerate() {
        dimensions(page)?;
        if page.page as usize != index || !layers.insert(Id::try_from(page.layer_id.clone())?) { return Err(CoreError::Invalid); }
        if kind == EditableDocumentKind::Pdf {
            let native = vw_pdf::PageGeometry::new(page.crop.clone().try_into().map_err(|_| CoreError::Invalid)?, page.quarter_turns as u8, vw_pdf::PdfLimits::default()).map_err(|_| CoreError::Invalid)?;
            if (native.width() - page.width).abs() > 0.001 || (native.height() - page.height).abs() > 0.001 { return Err(CoreError::Invalid); }
        } else if page.crop != [0.0, 0.0, page.width, page.height] || page.quarter_turns != 0 { return Err(CoreError::Invalid); }
    }
    Ok(())
}
#[uniffi::export]
pub async fn create_editable_document(options: CreateEditableDocument, cancellation: Arc<Cancellation>) -> Result<Arc<ProjectSession>> {
    if !Path::new(&options.path).is_absolute() || options.path.len() > 32767 || options.path.contains('\0')
        || options.source.is_empty() || options.source.len() as u64 > DOCUMENT_SOURCE_LIMIT
        || options.kind == EditableDocumentKind::Svg && options.source.len() > 4 * 1024 * 1024
        || options.title.len() > 4096 || options.now_ms < 0 { return Err(CoreError::Invalid); }
    validate_pages(options.kind, &options.pages)?;
    startup(move || {
        cancellation.check()?;
        let asset = AssetId::hash(&options.source);
        if asset.as_str() != options.source_blake3 { return Err(CoreError::Invalid); }
        let permit = SessionPermit::acquire()?;
        let project_id = Id::try_from(options.project_id)?; let doc_id = Id::try_from(options.document_id)?;
        let device = DeviceId::try_from(options.device_id)?;
        let mut project = Project::new(project_id, options.title.clone(), device.clone());
        let (width, height) = dimensions(&options.pages[0])?;
        let metadata_json=if options.kind==EditableDocumentKind::Svg { serde_json::json!({"software":"VisualWorkbench.SvgGeometry.1","description":serde_json::to_string(&[options.pages[0].width,options.pages[0].height]).map_err(|_| CoreError::Invalid)?}).to_string() } else { "{}".into() };
        project.assets.insert(asset.clone(), pb::AddAsset { asset_id: asset.to_string(), format: match options.kind { EditableDocumentKind::Pdf => "pdf", EditableDocumentKind::Svg => "svg" }.into(), width, height, orientation: 1, bit_depth: 8, has_alpha: options.kind == EditableDocumentKind::Svg, color_space: "untagged".into(), byte_size: options.source.len() as u64, source: "import".into(), metadata_json, ..Default::default() });
        let pages = if options.kind == EditableDocumentKind::Pdf { options.pages.iter().map(|page| PdfPage { page_index: page.page, crop_box: pb::RectD { x: page.crop[0], y: page.crop[1], w: page.crop[2] - page.crop[0], h: page.crop[3] - page.crop[1] }, rotation_degrees: page.quarter_turns as i32 * 90 }).collect() } else { vec![] };
        project.documents.insert(doc_id.clone(), Document { definition: pb::CreateDocument { document_id: Some(doc_id.to_proto()), kind: match options.kind { EditableDocumentKind::Pdf => pb::DocumentKind::Pdf, EditableDocumentKind::Svg => pb::DocumentKind::Svg } as i32, schema_version: 1, title: options.title, primary_asset_id: asset.to_string(), ..Default::default() }, pages, created_at_ms: options.now_ms });
        for page in options.pages {
            let layer = Id::try_from(page.layer_id)?;
            project.layers.insert(layer.clone(), Layer { definition: pb::CreateLayer { layer_id: Some(layer.to_proto()), document_id: Some(doc_id.to_proto()), page_index: if options.kind == EditableDocumentKind::Pdf { page.page as i32 } else { -1 }, name: format!("Annotations {}", page.page + 1), kind: "annotation".into(), order_key: "V".into() }, visible: true, locked: false, opacity: 1.0, blend: "normal".into() });
        }
        project.validate()?; cancellation.check()?;
        let (store, blobs) = crate::creation::create_complete(Path::new(&options.path), crate::creation::InitialProject { project, device, time: options.now_ms }, &options.source, &asset, &cancellation, |_| Ok(()))?;
        ProjectSession::from_store(store, blobs, permit)
    }).await
}

pub(crate) fn descriptor(state: &crate::project::ProjectState, document: &str) -> Result<Option<EditableDocumentDescriptor>> {
    let id = Id::try_from(document.to_owned())?; let project = state.project()?;
    let doc = project.documents.get(&id).ok_or(CoreError::Invalid)?;
    let kind = match pb::DocumentKind::try_from(doc.definition.kind).map_err(|_| CoreError::Invalid)? { pb::DocumentKind::Pdf => EditableDocumentKind::Pdf, pb::DocumentKind::Svg => EditableDocumentKind::Svg, _ => return Ok(None) };
    let asset = project.assets.get(&AssetId::try_from(doc.definition.primary_asset_id.clone())?).ok_or(CoreError::Invalid)?;
    let mut pages = Vec::new();
    let geometries: Vec<(u32, Vec<f64>, u32, f64, f64)> = if kind == EditableDocumentKind::Pdf {
        doc.pages.iter().map(|page| {
            let crop = vec![page.crop_box.x, page.crop_box.y, page.crop_box.x + page.crop_box.w, page.crop_box.y + page.crop_box.h];
            let turn = u32::try_from(page.rotation_degrees / 90).map_err(|_| CoreError::Invalid)?;
            let geometry = vw_pdf::PageGeometry::new(crop.clone().try_into().map_err(|_| CoreError::Invalid)?, turn as u8, vw_pdf::PdfLimits::default()).map_err(|_| CoreError::Invalid)?;
            Ok((page.page_index, crop, turn, geometry.width(), geometry.height()))
        }).collect::<Result<_>>()?
    } else {
        let metadata: serde_json::Value=serde_json::from_str(&asset.metadata_json).map_err(|_| CoreError::Invalid)?;
        if metadata.get("software").and_then(serde_json::Value::as_str)!=Some("VisualWorkbench.SvgGeometry.1") { return Err(CoreError::Unsupported); }
        let text=metadata.get("description").and_then(serde_json::Value::as_str).filter(|s| s.len()<=256).ok_or(CoreError::Invalid)?;
        let [width,height]:[f64;2]=serde_json::from_str(text).map_err(|_| CoreError::Invalid)?;
        if width.ceil()!=f64::from(asset.width) || height.ceil()!=f64::from(asset.height) { return Err(CoreError::Invalid); }
        vec![(0, vec![0.0, 0.0, width, height], 0, width, height)]
    };
    for (page, crop, quarter_turns, width, height) in geometries {
        let layer_id = project.layers.iter().find(|(_, layer)| layer.definition.document_id.as_ref() == Some(&id.to_proto()) && layer.definition.page_index == if kind == EditableDocumentKind::Pdf { page as i32 } else { -1 } && layer.definition.kind == "annotation").map(|(id, _)| id.to_string()).ok_or(CoreError::Invalid)?;
        pages.push(DocumentPageGeometry { page, width, height, crop, quarter_turns, layer_id });
    }
    pages.sort_by_key(|page| page.page); validate_pages(kind, &pages)?;
    Ok(Some(EditableDocumentDescriptor { document_id: document.to_owned(), source_asset_id: doc.definition.primary_asset_id.clone(), kind, pages, revision: state.info()? }))
}
#[uniffi::export]
impl ProjectSession {
    pub async fn editable_document_render(&self, document_id: String, page: u32, rectangle: Option<QueryRect>, cancellation: Arc<Cancellation>) -> Result<RenderList> {
        self.check_open()?; self.worker.call(move |state| {cancellation.check()?;let result=crate::queries::render_list_page(state,&document_id,page,rectangle)?;cancellation.check()?;Ok(result)}).await
    }
    pub async fn publish_editable_copy(&self, options: PublishEditableCopy, cancellation: Arc<Cancellation>) -> Result<String> {
        self.check_open()?;
        if options.bytes.is_empty() || options.bytes.len()>128*1024*1024 || !Path::new(&options.destination).is_absolute() || options.destination.len()>32767 || options.destination.contains('\0') { return Err(CoreError::Invalid); }
        self.worker.call(move |state| {
            cancellation.check()?;let descriptor=descriptor(state,&options.document_id)?.ok_or(CoreError::Unsupported)?;
            if descriptor.source_asset_id!=options.source_asset_id || descriptor.revision.project_id!=options.expected.project_id || descriptor.revision.host_seq!=options.expected.host_seq || descriptor.revision.state_hash!=options.expected.state_hash { return Err(CoreError::Invalid); }
            if blake3::hash(&options.bytes).to_hex().as_str()!=options.output_blake3 { return Err(CoreError::Invalid); }
            let destination=Path::new(&options.destination);let parent=destination.parent().ok_or(CoreError::Invalid)?.canonicalize()?;let destination=parent.join(destination.file_name().ok_or(CoreError::Invalid)?);
            let mut temporary=tempfile::NamedTempFile::new_in(&parent)?;
            use std::io::Write;temporary.write_all(&options.bytes)?;temporary.as_file().sync_all()?;cancellation.check()?;
            // No-clobber publication is the commit point. Later cancellation
            // never deletes a successfully published owner-selected copy.
            let published=temporary.persist_noclobber(&destination).map_err(|error| CoreError::from(error.error))?;published.sync_all()?;crate::creation::sync_directory(&parent)?;
            Ok(options.output_blake3)
        }).await
    }
    pub async fn editable_document_overlays(&self, document_id: String, page: u32, expected: ProjectInfo, cancellation: Arc<Cancellation>) -> Result<Vec<EditableOverlay>> {
        self.check_open()?; self.worker.call(move |state| {
            cancellation.check()?; let descriptor=descriptor(state,&document_id)?.ok_or(CoreError::Unsupported)?;
            if !descriptor.pages.iter().any(|item| item.page==page) || descriptor.revision.project_id!=expected.project_id || descriptor.revision.host_seq!=expected.host_seq || descriptor.revision.state_hash!=expected.state_hash { return Err(CoreError::Invalid); }
            let render=crate::queries::render_list_page(state,&document_id,page,None)?;
            let result=overlays::outlines(&render.items,page)?; cancellation.check()?; Ok(result)
        }).await
    }
    pub async fn editable_document_descriptor(&self, document_id: String, cancellation: Arc<Cancellation>) -> Result<Option<EditableDocumentDescriptor>> {
        self.check_open()?; self.worker.call(move |state| { cancellation.check()?; descriptor(state, &document_id) }).await
    }
    pub async fn editable_document_source(&self, document_id: String, expected: ProjectInfo, cancellation: Arc<Cancellation>) -> Result<EditableDocumentSource> {
        self.check_open()?; self.worker.call(move |state| {
            cancellation.check()?; let descriptor = descriptor(state, &document_id)?.ok_or(CoreError::Unsupported)?;
            if descriptor.revision.project_id != expected.project_id || descriptor.revision.host_seq != expected.host_seq || descriptor.revision.state_hash != expected.state_hash { return Err(CoreError::Invalid); }
            let asset = AssetId::try_from(descriptor.source_asset_id.clone())?;
            // Bounded read from one retained handle; hash the exact bytes adopted.
            let path = state.blobs.path(&asset)?; let file = std::fs::File::open(&path)?;
            let cap = if descriptor.kind == EditableDocumentKind::Svg { 4 * 1024 * 1024 } else { DOCUMENT_SOURCE_LIMIT };
            let length = file.metadata()?.len(); if length == 0 || length > cap { return Err(CoreError::Backpressure); }
            let mut bytes = Vec::new(); bytes.try_reserve_exact(length as usize).map_err(|_| CoreError::Backpressure)?;
            file.take(cap + 1).read_to_end(&mut bytes)?; cancellation.check()?;
            if bytes.len() as u64 > cap || AssetId::hash(&bytes) != asset || state.blobs.path(&asset)? != path { return Err(CoreError::Storage); }
            Ok(EditableDocumentSource { descriptor, bytes })
        }).await
    }
    pub async fn editable_document_page(&self, document_id: String, page: u32, cancellation: Arc<Cancellation>) -> Result<DocumentSnapshot> {
        self.check_open()?; self.worker.call(move |state| {
            cancellation.check()?; let descriptor = descriptor(state, &document_id)?.ok_or(CoreError::Unsupported)?;
            let geometry = descriptor.pages.get(page as usize).filter(|item| item.page == page).ok_or(CoreError::Invalid)?;
            let (width, height) = dimensions(geometry)?; let project = state.project()?; let id = Id::try_from(document_id.clone())?;
            let mut layers = project.layers.iter().filter(|(_, layer)| layer.definition.document_id.as_ref() == Some(&id.to_proto()) && layer.definition.page_index == if descriptor.kind == EditableDocumentKind::Pdf { page as i32 } else { -1 }).collect::<Vec<_>>();
            layers.sort_by(|(a, l), (b, r)| l.definition.order_key.cmp(&r.definition.order_key).then(a.cmp(b)));
            let layers = layers.into_iter().map(|(id, l)| LayerInfo { id: id.to_string(), name: l.definition.name.clone(), visible: l.visible, locked: l.locked, opacity: l.opacity, blend: l.blend.clone() }).collect();
            let render = crate::queries::render_list_page(state, &document_id, page, None)?;
            let result = DocumentSnapshot { document_id, title: project.documents.get(&id).ok_or(CoreError::Invalid)?.definition.title.clone(), width, height, bit_depth: 8, layers, render };
            crate::payload::document_bytes(&result)?; cancellation.check()?; Ok(result)
        }).await
    }
}

//! Materialized lookup tables. Canonical snapshots and the accepted log remain
//! the recovery authority; all projections change in that same transaction.
use crate::{
    StoreError,
    database::{decode, optional_uuid, sql_u64, uuid},
};
use rusqlite::{Connection, OptionalExtension, params};
use vw_model::{
    AssetId, Document, Group, Id, Instruction, Layer, Object, Project, ResultCandidate,
    SemanticSnapshot,
};
use vw_ops::Touched;
use vw_proto::{
    Message,
    v1::{self, object_state::Shape},
};

/// Rebuild every projection row from the project. Used when a project is
/// created, migrated or replaced by a checkpoint, and as the reference that
/// `verify` compares the stored rows with.
pub(crate) fn persist(db: &Connection, p: &Project, seq: i64, now: i64) -> Result<(), StoreError> {
    for asset in p.assets.values() {
        persist_asset(db, asset)?;
    }
    db.execute_batch("DELETE FROM results; DELETE FROM semantic_snapshots; DELETE FROM instructions; DELETE FROM objects; DELETE FROM mask_versions; DELETE FROM groups; DELETE FROM pdf_pages; DELETE FROM layers; DELETE FROM captures;")?;
    // Keep document identities for local package history and mark removed ones.
    db.execute(
        "UPDATE documents SET deleted_at=?1 WHERE deleted_at IS NULL",
        [now],
    )?;
    for (id, document) in &p.documents {
        write_document(db, id, document)?;
    }
    for (id, layer) in &p.layers {
        write_layer(db, id, layer)?;
    }
    for group in p.groups.values() {
        write_group(db, group)?;
    }
    for (id, object) in &p.objects {
        write_object(db, id, object, seq)?;
    }
    for (id, instruction) in &p.instructions {
        write_instruction(db, id, instruction)?;
    }
    for (id, snapshot) in &p.semantic_snapshots {
        write_semantic_snapshot(db, id, snapshot)?;
    }
    for (id, result) in &p.results {
        write_result(db, id, result)?;
    }
    for (id, versions) in &p.mask_versions {
        write_mask_versions(db, id, versions)?;
    }
    Ok(())
}

/// Bring the projection rows up to date after one accepted transaction by
/// writing only the entities it touched. The rows end up exactly as `persist`
/// would leave them, apart from `updated_seq` on objects the transaction did
/// not touch, which now keeps the sequence that last changed them.
pub(crate) fn persist_touched(
    db: &Connection,
    p: &Project,
    touched: &Touched,
    seq: i64,
    now: i64,
) -> Result<(), StoreError> {
    for id in &touched.assets {
        if let Some(asset) = p.assets.get(id) {
            persist_asset(db, asset)?;
        }
    }
    // New and changed rows first, parents before the rows that refer to them.
    for id in &touched.documents {
        match p.documents.get(id) {
            Some(document) => write_document(db, id, document)?,
            None => {
                // Keep the identity for local package history and mark it removed.
                remove_document_rows(db, id)?;
                db.prepare(
                    "UPDATE documents SET deleted_at=?1 WHERE document_id=?2 AND deleted_at IS NULL",
                )?
                .execute(params![now, id.as_str()])?;
            }
        }
    }
    for id in &touched.layers {
        if let Some(layer) = p.layers.get(id) {
            write_layer(db, id, layer)?;
        }
    }
    for id in &touched.groups {
        if let Some(group) = p.groups.get(id) {
            write_group(db, group)?;
        }
    }
    for id in &touched.objects {
        if let Some(object) = p.objects.get(id) {
            write_object(db, id, object, seq)?;
        }
    }
    for id in &touched.instructions {
        if let Some(instruction) = p.instructions.get(id) {
            write_instruction(db, id, instruction)?;
        }
    }
    for id in &touched.semantic_snapshots {
        if let Some(snapshot) = p.semantic_snapshots.get(id) {
            write_semantic_snapshot(db, id, snapshot)?;
        }
    }
    for id in &touched.results {
        if let Some(result) = p.results.get(id) {
            write_result(db, id, result)?;
        }
    }
    for id in &touched.mask_versions {
        if let Some(versions) = p.mask_versions.get(id) {
            write_mask_versions(db, id, versions)?;
        }
    }
    // Then removed rows, in the order `persist` clears the tables: a row goes
    // before the layer it refers to.
    let remove = |sql: &str, id: &str| -> Result<(), StoreError> {
        db.prepare(sql)?.execute([id])?;
        Ok(())
    };
    for id in &touched.results {
        if !p.results.contains_key(id) {
            remove("DELETE FROM results WHERE result_id=?1", id.as_str())?;
        }
    }
    for id in &touched.semantic_snapshots {
        if !p.semantic_snapshots.contains_key(id) {
            remove(
                "DELETE FROM semantic_snapshots WHERE snapshot_id=?1",
                id.as_str(),
            )?;
        }
    }
    for id in &touched.instructions {
        if !p.instructions.contains_key(id) {
            remove(
                "DELETE FROM instructions WHERE instruction_id=?1",
                id.as_str(),
            )?;
        }
    }
    for id in &touched.objects {
        if !p.objects.contains_key(id) {
            remove("DELETE FROM objects WHERE object_id=?1", id.as_str())?;
        }
    }
    for id in &touched.mask_versions {
        if !p.mask_versions.contains_key(id) {
            remove("DELETE FROM mask_versions WHERE object_id=?1", id.as_str())?;
        }
    }
    for id in &touched.groups {
        if !p.groups.contains_key(id) {
            remove("DELETE FROM groups WHERE group_id=?1", id.as_str())?;
        }
    }
    for id in &touched.layers {
        if !p.layers.contains_key(id) {
            remove("DELETE FROM layers WHERE layer_id=?1", id.as_str())?;
        }
    }
    Ok(())
}

// One function per table writes a row, for `persist` and `persist_touched`
// alike, so the two cannot disagree about a row's contents. Each replaces the
// row when it exists: a changed layer cannot be deleted and inserted again
// while objects still refer to it.

fn remove_document_rows(db: &Connection, id: &Id) -> Result<(), StoreError> {
    db.prepare("DELETE FROM pdf_pages WHERE document_id=?1")?
        .execute([id.as_str()])?;
    db.prepare("DELETE FROM captures WHERE document_id=?1")?
        .execute([id.as_str()])?;
    Ok(())
}

fn write_document(db: &Connection, id: &Id, document: &Document) -> Result<(), StoreError> {
    let d = &document.definition;
    let kind = match v1::DocumentKind::try_from(d.kind) {
        Ok(v1::DocumentKind::Image) => "image",
        Ok(v1::DocumentKind::Pdf) => "pdf",
        Ok(v1::DocumentKind::Svg) => "svg",
        Ok(v1::DocumentKind::Capture) => "capture",
        _ => return Err(StoreError::Invalid("document kind")),
    };
    db.prepare("INSERT INTO documents(document_id,kind,schema_version,title,primary_asset_id,created_at,deleted_at,state) VALUES(?1,?2,?3,?4,?5,?6,NULL,?7) ON CONFLICT(document_id) DO UPDATE SET kind=excluded.kind,schema_version=excluded.schema_version,title=excluded.title,primary_asset_id=excluded.primary_asset_id,created_at=excluded.created_at,deleted_at=NULL,state=excluded.state")?
        .execute(params![id.as_str(),kind,d.schema_version,d.title,optional_text(&d.primary_asset_id),document.created_at_ms,serde_json::to_vec(document)?])?;
    remove_document_rows(db, id)?;
    for page in &document.pages {
        db.prepare("INSERT INTO pdf_pages VALUES(?1,?2,?3)")?
            .execute(params![
                id.as_str(),
                page.page_index,
                serde_json::to_vec(page)?
            ])?;
    }
    if let Some(c) = &d.capture {
        let geometry = c
            .geometry
            .as_ref()
            .ok_or(StoreError::Invalid("capture geometry"))?;
        db.prepare("INSERT INTO captures(document_id,capture_session_id,frame_id,geometry,platform,app_name,window_title,lossless,degraded,captured_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?
            .execute(params![id.as_str(),uuid(c.capture_session_id.as_ref())?,c.frame_id.to_string(),geometry.encode_to_vec(),c.platform,c.app_name,c.window_title,c.lossless,c.degraded,c.captured_at_ms])?;
    }
    Ok(())
}

fn write_layer(db: &Connection, id: &Id, layer: &Layer) -> Result<(), StoreError> {
    let d = &layer.definition;
    db.prepare("INSERT INTO layers(layer_id,document_id,page_index,name,kind,order_key,visible,locked,opacity,blend) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(layer_id) DO UPDATE SET document_id=excluded.document_id,page_index=excluded.page_index,name=excluded.name,kind=excluded.kind,order_key=excluded.order_key,visible=excluded.visible,locked=excluded.locked,opacity=excluded.opacity,blend=excluded.blend")?
        .execute(params![id.as_str(),uuid(d.document_id.as_ref())?,d.page_index,d.name,d.kind,d.order_key,layer.visible,layer.locked,layer.opacity,layer.blend])?;
    Ok(())
}

fn write_group(db: &Connection, group: &Group) -> Result<(), StoreError> {
    db.prepare("INSERT INTO groups(group_id,document_id,parent_id) VALUES(?1,?2,?3) ON CONFLICT(group_id) DO UPDATE SET document_id=excluded.document_id,parent_id=excluded.parent_id")?
        .execute(params![
            group.id.as_str(),
            group.document_id.as_str(),
            group.parent.as_ref().map(|id| id.as_str())
        ])?;
    Ok(())
}

fn write_object(db: &Connection, id: &Id, object: &Object, seq: i64) -> Result<(), StoreError> {
    let o = &object.state;
    let (kind, marker) = shape_name(
        o.shape
            .as_ref()
            .ok_or(StoreError::Invalid("object shape"))?,
    );
    db.prepare("INSERT INTO objects(object_id,document_id,layer_id,kind,order_key,role,marker_number,group_id,state,updated_seq) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(object_id) DO UPDATE SET document_id=excluded.document_id,layer_id=excluded.layer_id,kind=excluded.kind,order_key=excluded.order_key,role=excluded.role,marker_number=excluded.marker_number,group_id=excluded.group_id,state=excluded.state,updated_seq=excluded.updated_seq")?
        .execute(params![id.as_str(),object.document_id.as_str(),uuid(o.layer_id.as_ref())?,kind,o.order_key,role(o.role)?,marker,optional_uuid(o.group_id.as_ref())?,o.encode_to_vec(),seq])?;
    Ok(())
}

fn write_instruction(
    db: &Connection,
    id: &Id,
    instruction: &Instruction,
) -> Result<(), StoreError> {
    let d = &instruction.definition;
    let targets = d
        .target_object_ids
        .iter()
        .map(|id| uuid(Some(id)))
        .collect::<Result<Vec<_>, _>>()?;
    db.prepare("INSERT INTO instructions(instruction_id,document_id,target_ids,role,text,entry_method,language,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(instruction_id) DO UPDATE SET document_id=excluded.document_id,target_ids=excluded.target_ids,role=excluded.role,text=excluded.text,entry_method=excluded.entry_method,language=excluded.language,updated_at=excluded.updated_at")?
        .execute(params![
            id.as_str(),
            uuid(d.document_id.as_ref())?,
            serde_json::to_string(&targets)?,
            role(d.role)?,
            d.text,
            d.entry_method,
            d.language,
            instruction.updated_at_ms
        ])?;
    Ok(())
}

fn write_semantic_snapshot(
    db: &Connection,
    id: &Id,
    snapshot: &SemanticSnapshot,
) -> Result<(), StoreError> {
    let d = &snapshot.definition;
    db.prepare("INSERT INTO semantic_snapshots(snapshot_id,document_id,platform,frame_delta_ms,elements,created_at,capture_session_id,frame_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(snapshot_id) DO UPDATE SET document_id=excluded.document_id,platform=excluded.platform,frame_delta_ms=excluded.frame_delta_ms,elements=excluded.elements,created_at=excluded.created_at,capture_session_id=excluded.capture_session_id,frame_id=excluded.frame_id")?
        .execute(params![id.as_str(),uuid(d.document_id.as_ref())?,d.platform,d.frame_delta_ms,d.elements_json_zstd,snapshot.created_at_ms,snapshot.capture_session_id.as_ref().map(|v|v.as_str()),snapshot.frame_id.map(|v|v.to_string())])?;
    Ok(())
}

fn write_result(db: &Connection, id: &Id, result: &ResultCandidate) -> Result<(), StoreError> {
    let d = &result.definition;
    db.prepare("INSERT INTO results(result_id,document_id,package_id,provider,model,request_json,output_asset_id,composite_asset_id,proof_json,metrics_json,cost_estimate_usd,status,acceptance_mask_asset_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(result_id) DO UPDATE SET document_id=excluded.document_id,package_id=excluded.package_id,provider=excluded.provider,model=excluded.model,request_json=excluded.request_json,output_asset_id=excluded.output_asset_id,composite_asset_id=excluded.composite_asset_id,proof_json=excluded.proof_json,metrics_json=excluded.metrics_json,cost_estimate_usd=excluded.cost_estimate_usd,status=excluded.status,acceptance_mask_asset_id=excluded.acceptance_mask_asset_id,created_at=excluded.created_at")?
        .execute(params![id.as_str(),uuid(d.document_id.as_ref())?,optional_uuid(d.package_id.as_ref())?,d.provider,d.model,d.request_json,optional_text(&d.output_asset_id),optional_text(&d.composite_asset_id),d.proof_json,d.metrics_json,d.cost_estimate_usd,result.status,result.acceptance_mask_asset_id.as_ref().map(|v|v.as_str()),result.created_at_ms])?;
    Ok(())
}

fn write_mask_versions(
    db: &Connection,
    id: &Id,
    versions: &[v1::MaskOp],
) -> Result<(), StoreError> {
    db.prepare("INSERT INTO mask_versions(object_id,state) VALUES(?1,?2) ON CONFLICT(object_id) DO UPDATE SET state=excluded.state")?
        .execute(params![id.as_str(), serde_json::to_vec(versions)?])?;
    Ok(())
}

pub(crate) fn verify_assets(db: &Connection, p: &Project) -> Result<(), StoreError> {
    let mut query = db.prepare("SELECT asset_id,byte_size,definition FROM assets")?;
    let mut rows = query.query([])?;
    let mut visible = 0;
    while let Some(row) = rows.next()? {
        let id = AssetId::try_from(row.get::<_, String>(0)?)?;
        let asset: v1::AddAsset = decode(&row.get::<_, Vec<u8>>(2)?)?;
        vw_model::validate_asset(&asset)?;
        let matching:i64=db.query_row("SELECT COUNT(*) FROM assets WHERE asset_id=?1 AND format=?2 AND width=?3 AND height=?4 AND orientation=?5 AND bit_depth=?6 AND has_alpha=?7 AND color_space=?8 AND COALESCE(icc_profile,x'')=?9 AND byte_size=?10 AND source=?11 AND COALESCE(captured_at,0)=?12 AND metadata_json=?13",params![asset.asset_id,asset.format,asset.width,asset.height,asset.orientation,asset.bit_depth,asset.has_alpha,asset.color_space,asset.icc_profile,sql_u64(asset.byte_size)?,asset.source,asset.captured_at_ms,asset.metadata_json],|r|r.get(0))?;
        if matching != 1 {
            return Err(StoreError::Corrupt("asset metadata projection"));
        }
        if asset.asset_id != id.as_str() || sql_u64(asset.byte_size)? != row.get::<_, i64>(1)? {
            return Err(StoreError::Corrupt("asset inventory"));
        }
        if let Some(current) = p.assets.get(&id) {
            if current != &asset {
                return Err(StoreError::Corrupt("asset projection"));
            }
            visible += 1;
        }
    }
    if visible != p.assets.len() {
        return Err(StoreError::Corrupt("missing asset metadata"));
    }
    Ok(())
}

/// Compare all current materialized rows to an independently populated in-memory
/// database. Historical asset/document inventory and local settings stay separate.
pub(crate) fn verify(db: &Connection, p: &Project, seq: i64) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(crate::migrations::SCHEMA)?;
    persist(&reference, p, seq, 0)?;
    for table in [
        "documents",
        "captures",
        "layers",
        "objects",
        "groups",
        "pdf_pages",
        "mask_versions",
        "instructions",
        "semantic_snapshots",
        "results",
    ] {
        // Table and column names below come solely from the embedded schema.
        let columns = reference
            .prepare(&format!("PRAGMA table_info({table})"))?
            .query_map([], |r| r.get::<_, String>(1))?
            .collect::<Result<Vec<_>, _>>()?;
        let columns = columns
            .iter()
            .filter(|c| c.as_str() != "updated_seq")
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        let filter = if table == "documents" {
            " WHERE deleted_at IS NULL"
        } else {
            ""
        };
        let sql = format!("SELECT {columns} FROM {table}{filter} ORDER BY 1,2");
        let mut expected = reference.prepare(&sql)?;
        let mut actual = db.prepare(&sql)?;
        let count = expected.column_count();
        let mut left = expected.query([])?;
        let mut right = actual.query([])?;
        loop {
            match (left.next()?, right.next()?) {
                (None, None) => break,
                (Some(a), Some(b)) => {
                    for index in 0..count {
                        if a.get::<_, rusqlite::types::Value>(index)?
                            != b.get::<_, rusqlite::types::Value>(index)?
                        {
                            return Err(StoreError::Corrupt("materialized projection"));
                        }
                    }
                }
                _ => return Err(StoreError::Corrupt("materialized projection")),
            }
        }
    }
    if db.query_row(
        "SELECT COUNT(*) FROM objects WHERE updated_seq<0 OR updated_seq>?1",
        [seq],
        |r| r.get::<_, i64>(0),
    )? != 0
    {
        return Err(StoreError::Corrupt("object sequence"));
    }
    Ok(())
}
fn optional_text(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}
fn role(role: i32) -> Result<&'static str, StoreError> {
    match v1::Role::try_from(role) {
        Ok(v1::Role::None) => Ok("none"),
        Ok(v1::Role::Change) => Ok("change"),
        Ok(v1::Role::Preserve) => Ok("preserve"),
        Ok(v1::Role::Reference) => Ok("reference"),
        Ok(v1::Role::Explain) => Ok("explain"),
        _ => Err(StoreError::Invalid("role")),
    }
}
fn shape_name(shape: &Shape) -> (&'static str, Option<u32>) {
    match shape {
        Shape::Stroke(_) => ("stroke", None),
        Shape::Line(_) => ("line", None),
        Shape::Arrow(_) => ("arrow", None),
        Shape::Rect(_) => ("rect", None),
        Shape::Ellipse(_) => ("ellipse", None),
        Shape::Polygon(_) => ("polygon", None),
        Shape::Text(_) => ("text", None),
        Shape::Marker(m) => ("marker", Some(m.number)),
        Shape::SelectionVector(_) => ("selection_vector", None),
        Shape::SelectionRaster(_) => ("selection_raster", None),
        Shape::Crop(_) => ("crop", None),
        Shape::ResultId(_) => ("result", None),
        Shape::Adjustment(_) => ("adjustment", None),
    }
}

pub(crate) fn persist_asset(db: &Connection, asset: &v1::AddAsset) -> Result<(), StoreError> {
    vw_model::validate_asset(asset)?;
    if asset.encoded_len() > crate::database::MAX_PAYLOAD {
        return Err(StoreError::Invalid("asset payload size"));
    }
    let encoded = asset.encode_to_vec();
    let prior: Option<Option<Vec<u8>>> = db
        .query_row(
            "SELECT definition FROM assets WHERE asset_id=?1",
            [&asset.asset_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(Some(prior)) = prior.as_ref()
        && *prior != encoded
    {
        return Err(StoreError::Corrupt("immutable asset metadata"));
    }
    if prior.is_none() {
        db.execute("INSERT INTO assets(asset_id,format,width,height,orientation,bit_depth,has_alpha,color_space,icc_profile,byte_size,source,captured_at,metadata_json,local_state,definition) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'remote',?14)",
            params![asset.asset_id,asset.format,asset.width,asset.height,asset.orientation,asset.bit_depth,asset.has_alpha,asset.color_space,asset.icc_profile,sql_u64(asset.byte_size)?,asset.source,asset.captured_at_ms,asset.metadata_json,encoded])?;
    } else if matches!(prior, Some(None)) {
        db.execute(
            "UPDATE assets SET definition=?1 WHERE asset_id=?2",
            params![encoded, asset.asset_id],
        )?;
    }
    Ok(())
}

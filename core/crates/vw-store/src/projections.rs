//! Materialized lookup tables. Canonical snapshots and the accepted log remain
//! the recovery authority; all projections change in that same transaction.
use crate::{
    StoreError,
    database::{decode, optional_uuid, sql_u64, uuid},
};
use rusqlite::{Connection, OptionalExtension, params};
use vw_model::{AssetId, Project};
use vw_proto::{
    Message,
    v1::{self, object_state::Shape},
};

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
        let d = &document.definition;
        let kind = match v1::DocumentKind::try_from(d.kind) {
            Ok(v1::DocumentKind::Image) => "image",
            Ok(v1::DocumentKind::Pdf) => "pdf",
            Ok(v1::DocumentKind::Svg) => "svg",
            Ok(v1::DocumentKind::Capture) => "capture",
            _ => return Err(StoreError::Invalid("document kind")),
        };
        db.execute("INSERT INTO documents(document_id,kind,schema_version,title,primary_asset_id,created_at,deleted_at,state) VALUES(?1,?2,?3,?4,?5,?6,NULL,?7) ON CONFLICT(document_id) DO UPDATE SET kind=excluded.kind,schema_version=excluded.schema_version,title=excluded.title,primary_asset_id=excluded.primary_asset_id,created_at=excluded.created_at,deleted_at=NULL,state=excluded.state",
            params![id.as_str(),kind,d.schema_version,d.title,optional_text(&d.primary_asset_id),document.created_at_ms,serde_json::to_vec(document)?])?;
        for page in &document.pages {
            db.execute(
                "INSERT INTO pdf_pages VALUES(?1,?2,?3)",
                params![id.as_str(), page.page_index, serde_json::to_vec(page)?],
            )?;
        }
        if let Some(c) = &d.capture {
            let geometry = c
                .geometry
                .as_ref()
                .ok_or(StoreError::Invalid("capture geometry"))?;
            db.execute("INSERT INTO captures(document_id,capture_session_id,frame_id,geometry,platform,app_name,window_title,lossless,degraded,captured_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![id.as_str(),uuid(c.capture_session_id.as_ref())?,c.frame_id.to_string(),geometry.encode_to_vec(),c.platform,c.app_name,c.window_title,c.lossless,c.degraded,c.captured_at_ms])?;
        }
    }
    for (id, layer) in &p.layers {
        let d = &layer.definition;
        db.execute("INSERT INTO layers(layer_id,document_id,page_index,name,kind,order_key,visible,locked,opacity,blend) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id.as_str(),uuid(d.document_id.as_ref())?,d.page_index,d.name,d.kind,d.order_key,layer.visible,layer.locked,layer.opacity,layer.blend])?;
    }
    for group in p.groups.values() {
        db.execute(
            "INSERT INTO groups VALUES(?1,?2,?3)",
            params![
                group.id.as_str(),
                group.document_id.as_str(),
                group.parent.as_ref().map(|id| id.as_str())
            ],
        )?;
    }
    for (id, object) in &p.objects {
        let o = &object.state;
        let (kind, marker) = shape_name(
            o.shape
                .as_ref()
                .ok_or(StoreError::Invalid("object shape"))?,
        );
        db.execute("INSERT INTO objects(object_id,document_id,layer_id,kind,order_key,role,marker_number,group_id,state,updated_seq) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![id.as_str(),object.document_id.as_str(),uuid(o.layer_id.as_ref())?,kind,o.order_key,role(o.role)?,marker,optional_uuid(o.group_id.as_ref())?,o.encode_to_vec(),seq])?;
    }
    for (id, instruction) in &p.instructions {
        let d = &instruction.definition;
        let targets = d
            .target_object_ids
            .iter()
            .map(|id| uuid(Some(id)))
            .collect::<Result<Vec<_>, _>>()?;
        db.execute(
            "INSERT INTO instructions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![
                id.as_str(),
                uuid(d.document_id.as_ref())?,
                serde_json::to_string(&targets)?,
                role(d.role)?,
                d.text,
                d.entry_method,
                d.language,
                instruction.updated_at_ms
            ],
        )?;
    }
    for (id, snapshot) in &p.semantic_snapshots {
        let d = &snapshot.definition;
        db.execute("INSERT INTO semantic_snapshots(snapshot_id,document_id,platform,frame_delta_ms,elements,created_at,capture_session_id,frame_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id.as_str(),uuid(d.document_id.as_ref())?,d.platform,d.frame_delta_ms,d.elements_json_zstd,snapshot.created_at_ms,snapshot.capture_session_id.as_ref().map(|v|v.as_str()),snapshot.frame_id.map(|v|v.to_string())])?;
    }
    for (id, result) in &p.results {
        let d = &result.definition;
        db.execute("INSERT INTO results(result_id,document_id,package_id,provider,model,request_json,output_asset_id,composite_asset_id,proof_json,metrics_json,cost_estimate_usd,status,acceptance_mask_asset_id,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![id.as_str(),uuid(d.document_id.as_ref())?,optional_uuid(d.package_id.as_ref())?,d.provider,d.model,d.request_json,optional_text(&d.output_asset_id),optional_text(&d.composite_asset_id),d.proof_json,d.metrics_json,d.cost_estimate_usd,result.status,result.acceptance_mask_asset_id.as_ref().map(|v|v.as_str()),result.created_at_ms])?;
    }
    for (id, versions) in &p.mask_versions {
        db.execute(
            "INSERT INTO mask_versions VALUES(?1,?2)",
            params![id.as_str(), serde_json::to_vec(versions)?],
        )?;
    }
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

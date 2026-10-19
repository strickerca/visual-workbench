use super::*;
use vw_model::DeviceId;

fn project() -> Project {
    let id = Id::from_parts(1, [1; 10]).expect("fixture id");
    Project::new(id, "bounded region".into(), DeviceId::from_bytes([2; 16]))
}

#[test]
fn empty_project_needs_no_result_decode_reservation() {
    let project = project();
    let document = Id::from_parts(3, [3; 10]).expect("fixture document");
    assert!(matches!(
        visible_result_workspace(&project, &document),
        Ok(0)
    ));
}

#[test]
fn result_resolver_refuses_oversized_original_before_blob_access() {
    let root = tempfile::tempdir().expect("private fixture directory");
    let blobs = vw_store::BlobStore::new(root.path()).expect("fixture blob store");
    let mut project = project();
    let id = AssetId::hash(b"oversized result fixture");
    project.assets.insert(
        id.clone(),
        vw_proto::v1::AddAsset {
            asset_id: id.to_string(),
            byte_size: MAX_RESULT_BYTES + 1,
            ..Default::default()
        },
    );
    let pool = SessionBudget::new(1024).expect("fixture pool");
    let charge = pool
        .reserve(MemoryKind::CodecWorkspace, 1)
        .expect("fixture charge");
    let snapshot = Snapshot {
        project,
        blobs,
        document: Id::from_parts(4, [4; 10]).expect("fixture document"),
        _charge: charge,
    };
    let cancel = Cancellation::new();
    let resolver = ResultAssets {
        snapshot: &snapshot,
        cancel: &cancel,
        budget: 1024,
    };
    assert!(matches!(
        resolver.image(id.as_str()),
        Err(RasterError::Unsupported("bounded result original"))
    ));
    drop(resolver);
    drop(snapshot);
    assert_eq!(pool.used_bytes(), 0);
}

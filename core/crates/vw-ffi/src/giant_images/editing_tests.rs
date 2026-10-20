use super::*;
use vw_model::DeviceId;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn project() -> TestResult<Project> {
    let id = Id::from_parts(1, [1; 10])?;
    Ok(Project::new(
        id,
        "bounded region".into(),
        DeviceId::from_bytes([2; 16]),
    ))
}

#[test]
fn empty_project_needs_no_result_decode_reservation() -> TestResult {
    let project = project()?;
    let document = Id::from_parts(3, [3; 10])?;
    assert!(matches!(
        visible_result_workspace(&project, &document),
        Ok(0)
    ));
    Ok(())
}

#[test]
fn result_resolver_refuses_oversized_original_before_blob_access() -> TestResult {
    let root = tempfile::tempdir()?;
    let blobs = vw_store::BlobStore::new(root.path())?;
    let mut project = project()?;
    let id = AssetId::hash(b"oversized result fixture");
    project.assets.insert(
        id.clone(),
        vw_proto::v1::AddAsset {
            asset_id: id.to_string(),
            byte_size: MAX_RESULT_BYTES + 1,
            ..Default::default()
        },
    );
    let pool = SessionBudget::new(1024)?;
    let charge = pool.reserve(MemoryKind::CodecWorkspace, 1)?;
    let snapshot = Snapshot {
        project,
        blobs,
        document: Id::from_parts(4, [4; 10])?,
        _charge: charge,
    };
    let cancel = Cancellation::new();
    {
        let resolver = ResultAssets {
            snapshot: &snapshot,
            cancel: &cancel,
            budget: 1024,
        };
        assert!(matches!(
            resolver.image(id.as_str()),
            Err(RasterError::Unsupported("bounded result original"))
        ));
    }
    drop(snapshot);
    assert_eq!(pool.used_bytes(), 0);
    Ok(())
}

//! A real generated MP4 follows the same immutable original/archive path.
#![allow(clippy::unwrap_used)]
use vw_model::{AssetId, DeviceId, Id, Project};
use vw_proto::v1;
use vw_store::{BlobStore, ImportLimits, ProjectStore, export_vwbz, import_vwbz};

#[test]
fn generated_one_second_mp4_roundtrips_database_blob_and_archive() {
    let bytes = include_bytes!("fixtures/short.mp4");
    assert_eq!(&bytes[4..8], b"ftyp");
    let id = AssetId::hash(bytes);
    #[cfg(target_os = "android")]
    let temp = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    #[cfg(not(target_os = "android"))]
    let temp = tempfile::tempdir().unwrap();
    let owner = DeviceId::from_bytes([0x32; 16]);
    let mut project = Project::new(
        Id::from_parts(1_700_000_000_000, [0x15; 10]).unwrap(),
        "Generated video fixture".into(),
        owner.clone(),
    );
    project.assets.insert(
        id.clone(),
        v1::AddAsset {
            asset_id: id.to_string(),
            format: "mp4".into(),
            width: 64,
            height: 64,
            orientation: 1,
            bit_depth: 8,
            color_space: "BT.709".into(),
            byte_size: bytes.len() as u64,
            source: "import".into(),
            metadata_json: "{}".into(),
            ..Default::default()
        },
    );
    let path = temp.path().join("video.vwb");
    let store = ProjectStore::create(&path, project.clone(), owner, 0).unwrap();
    let blobs = BlobStore::new(&path).unwrap();
    assert_eq!(blobs.put(bytes).unwrap(), id);
    assert_eq!(blobs.read(&id).unwrap(), bytes);
    drop(store);
    let mut reopened = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.project(), &project);
    let archive = temp.path().join("video.vwbz");
    let report = export_vwbz(&mut reopened, &archive).unwrap();
    assert_eq!(report.blobs, 1);
    let imported_path = temp.path().join("imported.vwb");
    let imported = import_vwbz(&archive, &imported_path, ImportLimits::default()).unwrap();
    assert_eq!(
        imported.project().state_hash().unwrap(),
        project.state_hash().unwrap()
    );
    assert_eq!(
        BlobStore::new(&imported_path).unwrap().read(&id).unwrap(),
        bytes
    );
    println!("T1.03_MP4_ROUNDTRIP bytes={} blake3={id}", bytes.len());
    drop(imported);
    drop(reopened);
    temp.close().unwrap();
}

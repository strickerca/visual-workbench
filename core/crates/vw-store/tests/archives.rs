//! Synthetic archive fixtures, including deliberately malformed ZIP metadata.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use tempfile::TempDir;
use vw_model::{AssetId, DeviceId, Document, Id, Project};
use vw_proto::{Message, v1 as pb};
use vw_store::{BlobStore, ImportLimits, ProjectStore, export_vwbz, import_vwbz};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const LABEL: &str = "SYNTHETIC_DEVICE_LABEL_NOT_FOR_EXPORT_4CB439F971";
const ORIGINAL: &[u8] = b"synthetic immutable archive original 58c2c7a11a0f";

fn temporary() -> TempDir {
    let mut builder = tempfile::Builder::new();
    builder.prefix("vw-archives-test-");
    #[cfg(target_os = "android")]
    {
        builder
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        builder.tempdir().unwrap()
    }
}

fn id(number: u8) -> Id {
    Id::from_parts(1_700_000_000_000 + u64::from(number), [number; 10]).unwrap()
}

fn device() -> DeviceId {
    DeviceId::from_bytes([1; 16])
}

fn metadata(id: &AssetId, size: u64) -> pb::AddAsset {
    pb::AddAsset {
        asset_id: id.to_string(),
        format: "png".into(),
        width: 1,
        height: 1,
        orientation: 1,
        bit_depth: 8,
        has_alpha: true,
        color_space: "sRGB".into(),
        byte_size: size,
        source: "import".into(),
        metadata_json: "{}".into(),
        ..Default::default()
    }
}

fn fixture(root: &Path) -> (ProjectStore, AssetId) {
    let asset = AssetId::hash(ORIGINAL);
    let mut project = Project::new(id(1), "Archive fixture".into(), device());
    project
        .assets
        .insert(asset.clone(), metadata(&asset, ORIGINAL.len() as u64));
    project.documents.insert(
        id(2),
        Document {
            definition: pb::CreateDocument {
                document_id: Some(id(2).to_proto()),
                kind: pb::DocumentKind::Image as i32,
                schema_version: 1,
                title: "Synthetic archive image".into(),
                primary_asset_id: asset.to_string(),
                capture: None,
            },
            pages: Vec::new(),
            created_at_ms: 1_700_000_000_000,
        },
    );
    let path = root.join("source.vwb");
    let store = ProjectStore::create(&path, project, device(), 1_700_000_000_000).unwrap();
    assert_eq!(BlobStore::new(&path).unwrap().put(ORIGINAL).unwrap(), asset);
    let connection = rusqlite::Connection::open(path.join("project.sqlite")).unwrap();
    connection
        .execute(
            "INSERT INTO devices(device_id, label, platform, lamport) VALUES (?1, ?2, 'windows', 0)
         ON CONFLICT(device_id) DO UPDATE SET label = excluded.label",
            rusqlite::params![device().to_string(), LABEL],
        )
        .unwrap();
    drop(connection);
    (store, asset)
}

fn blob_name(id: &AssetId) -> String {
    format!("blobs/{}/{}/{}", &id.as_str()[..2], &id.as_str()[2..4], id)
}

fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).unwrap();
            (name, bytes)
        })
        .collect()
}

fn clean_staging(root: &Path) {
    assert!(fs::read_dir(root).unwrap().all(|entry| {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        !name.starts_with(".vwb-import-")
            && !name.starts_with(".vwb-export-")
            && !name.starts_with(".vwb-archive-")
    }));
}

#[test]
fn roundtrip_preserves_state_hash_blobs_and_source_but_strips_raw_labels_and_cache() {
    let temporary = temporary();
    let (mut source, asset) = fixture(temporary.path());
    let source_path = temporary.path().join("source.vwb");
    fs::create_dir_all(source_path.join("cache")).unwrap();
    fs::write(
        source_path.join("cache/private-derived-data"),
        b"DERIVED-CACHE-FIXTURE",
    )
    .unwrap();
    let before = source.project().state_hash().unwrap();
    let archive = temporary.path().join("backup.vwbz");
    let report = export_vwbz(&mut source, &archive).unwrap();
    assert_eq!(report.entries, 2);
    assert_eq!(report.blobs, 1);
    assert_eq!(report.state_hash, before);
    assert!(report.uncompressed_bytes > ORIGINAL.len() as u64);
    let raw = fs::read(&archive).unwrap();
    assert!(
        !raw.windows(LABEL.len())
            .any(|window| window == LABEL.as_bytes())
    );
    assert!(
        !raw.windows(21)
            .any(|window| window == b"DERIVED-CACHE-FIXTURE")
    );
    let saved = entries(&archive);
    assert_eq!(saved.len(), 2);
    assert!(
        saved
            .iter()
            .all(|(name, _)| name == "project.sqlite" || name == &blob_name(&asset))
    );
    let restored_path = temporary.path().join("restored.vwb");
    let restored = import_vwbz(&archive, &restored_path, ImportLimits::default()).unwrap();
    assert_eq!(restored.project().state_hash().unwrap(), before);
    assert_eq!(restored.revision().unwrap(), source.revision().unwrap());
    assert_eq!(
        BlobStore::new(&restored_path)
            .unwrap()
            .read(&asset)
            .unwrap(),
        ORIGINAL
    );
    let connection = rusqlite::Connection::open(restored_path.join("project.sqlite")).unwrap();
    let count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM devices WHERE label IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    drop(connection);
    let connection = rusqlite::Connection::open(source_path.join("project.sqlite")).unwrap();
    let label: String = connection
        .query_row(
            "SELECT label FROM devices WHERE device_id = ?1",
            [device().to_string()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(label, LABEL);
    assert_eq!(source.project().state_hash().unwrap(), before);
    drop(connection);
    drop(restored);
    drop(source);
    clean_staging(temporary.path());
    temporary.close().unwrap();
}

#[test]
fn immutable_inventory_exports_historical_assets_outside_the_current_model() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let bytes = b"synthetic historical original";
    let historical = BlobStore::new(&temporary.path().join("source.vwb"))
        .unwrap()
        .put(bytes)
        .unwrap();
    assert!(!source.project().assets.contains_key(&historical));
    let connection =
        rusqlite::Connection::open(temporary.path().join("source.vwb/project.sqlite")).unwrap();
    connection.execute(
        "INSERT INTO assets(asset_id,format,width,height,orientation,bit_depth,has_alpha,color_space,byte_size,source,metadata_json,definition,icc_profile,captured_at)
         VALUES (?1,'png',1,1,1,8,1,'sRGB',?2,'import','{}',?3,x'',0)",
        rusqlite::params![historical.to_string(), bytes.len() as i64, metadata(&historical, bytes.len() as u64).encode_to_vec()],
    ).unwrap();
    drop(connection);
    let archive = temporary.path().join("history.vwbz");
    assert_eq!(export_vwbz(&mut source, &archive).unwrap().blobs, 2);
    assert!(
        entries(&archive)
            .iter()
            .any(|(name, data)| name == &blob_name(&historical) && data == bytes)
    );
    let destination = temporary.path().join("history.vwb");
    let restored = import_vwbz(&archive, &destination, ImportLimits::default()).unwrap();
    assert_eq!(
        restored.project().state_hash().unwrap(),
        source.project().state_hash().unwrap()
    );
    assert_eq!(
        BlobStore::new(&destination)
            .unwrap()
            .read(&historical)
            .unwrap(),
        bytes
    );
    drop(restored);
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn missing_or_corrupt_originals_abort_export_without_publishing() {
    for corrupt in [false, true] {
        let temporary = temporary();
        let (mut source, asset) = fixture(temporary.path());
        let path = BlobStore::new(&temporary.path().join("source.vwb"))
            .unwrap()
            .path(&asset)
            .unwrap();
        if corrupt {
            fs::write(&path, b"corrupt").unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        let destination = temporary.path().join("must-not-exist.vwbz");
        assert!(export_vwbz(&mut source, &destination).is_err());
        assert!(!destination.exists());
        clean_staging(temporary.path());
        drop(source);
        temporary.close().unwrap();
    }
}

#[test]
fn existing_files_and_directories_are_preserved_for_import_and_export() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let archive = temporary.path().join("valid.vwbz");
    export_vwbz(&mut source, &archive).unwrap();
    let previous = fs::read(&archive).unwrap();
    assert!(export_vwbz(&mut source, &archive).is_err());
    assert_eq!(fs::read(&archive).unwrap(), previous);
    let existing = temporary.path().join("existing.vwb");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("keep"), b"unrelated fixture").unwrap();
    assert!(import_vwbz(&archive, &existing, ImportLimits::default()).is_err());
    assert_eq!(
        fs::read(existing.join("keep")).unwrap(),
        b"unrelated fixture"
    );
    let empty = temporary.path().join("existing-empty.vwb");
    fs::create_dir(&empty).unwrap();
    assert!(import_vwbz(&archive, &empty, ImportLimits::default()).is_err());
    assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
    let file = temporary.path().join("existing-file");
    fs::write(&file, b"file fixture").unwrap();
    assert!(import_vwbz(&archive, &file, ImportLimits::default()).is_err());
    assert_eq!(fs::read(&file).unwrap(), b"file fixture");
    clean_staging(temporary.path());
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn missing_corrupt_and_unreferenced_blob_members_are_rejected_atomically() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let archive = temporary.path().join("valid.vwbz");
    export_vwbz(&mut source, &archive).unwrap();
    let original = entries(&archive);
    for case in 0..4 {
        let mut files = original.clone();
        match case {
            0 => files.retain(|(name, _)| name == "project.sqlite"),
            1 => {
                files
                    .iter_mut()
                    .find(|(name, _)| name != "project.sqlite")
                    .unwrap()
                    .1 = b"different content with valid CRC".to_vec()
            }
            2 => {
                let data = b"unreferenced original".to_vec();
                files.push((blob_name(&AssetId::hash(&data)), data));
            }
            _ => {
                files
                    .iter_mut()
                    .find(|(name, _)| name == "project.sqlite")
                    .unwrap()
                    .1 = b"not a SQLite database".to_vec()
            }
        }
        let malformed = temporary.path().join(format!("invalid-{case}.vwbz"));
        write_raw(&malformed, &files, None, None);
        let destination = temporary.path().join(format!("invalid-{case}.vwb"));
        assert!(import_vwbz(&malformed, &destination, ImportLimits::default()).is_err());
        assert!(!destination.exists());
        clean_staging(temporary.path());
    }
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn traversal_absolute_paths_sidecars_directories_and_duplicate_names_are_rejected() {
    let temporary = temporary();
    let bad_names = [
        "../project.sqlite",
        "/project.sqlite",
        "C:/project.sqlite",
        "project.sqlite:stream",
        "project.sqlite-wal",
        "cache/private",
        "blobs/aa/bb/not-a-hash",
        "project.sqlite/",
        "blobs\\aa\\bb\\data",
    ];
    for (index, name) in bad_names.iter().enumerate() {
        let malformed = temporary.path().join(format!("name-{index}.vwbz"));
        write_raw(
            &malformed,
            &[
                ("project.sqlite".into(), vec![0]),
                ((*name).into(), vec![1]),
            ],
            None,
            None,
        );
        let destination = temporary.path().join(format!("name-{index}.vwb"));
        assert!(import_vwbz(&malformed, &destination, ImportLimits::default()).is_err());
        assert!(!destination.exists());
    }
    let duplicate = temporary.path().join("duplicate.vwbz");
    write_raw(
        &duplicate,
        &[
            ("project.sqlite".into(), vec![0]),
            ("project.sqlite".into(), vec![1]),
        ],
        None,
        None,
    );
    let result = import_vwbz(
        &duplicate,
        &temporary.path().join("duplicate.vwb"),
        ImportLimits::default(),
    );
    assert!(matches!(
        result,
        Err(vw_store::StoreError::Invalid("duplicate archive member"))
    ));
    assert!(!temporary.path().join("project.sqlite").exists());
    clean_staging(temporary.path());
    temporary.close().unwrap();
}

#[test]
fn symlink_special_file_and_crc_mismatch_are_rejected() {
    let temporary = temporary();
    for (index, mode) in [0o120777_u32, 0o020600, 0o040700].into_iter().enumerate() {
        let archive = temporary.path().join(format!("mode-{index}.vwbz"));
        write_raw(
            &archive,
            &[("project.sqlite".into(), b"../outside".to_vec())],
            Some(mode),
            None,
        );
        let result = import_vwbz(
            &archive,
            &temporary.path().join(format!("mode-{index}.vwb")),
            ImportLimits::default(),
        );
        assert!(matches!(
            result,
            Err(vw_store::StoreError::Invalid("archive member type"))
        ));
    }
    let (mut source, _) = fixture(temporary.path());
    let archive = temporary.path().join("valid.vwbz");
    export_vwbz(&mut source, &archive).unwrap();
    let mut bytes = fs::read(&archive).unwrap();
    let offset = bytes
        .windows(ORIGINAL.len())
        .position(|window| window == ORIGINAL)
        .unwrap();
    bytes[offset] ^= 1;
    let bad_crc = temporary.path().join("bad-crc.vwbz");
    fs::write(&bad_crc, bytes).unwrap();
    let destination = temporary.path().join("bad-crc.vwb");
    assert!(import_vwbz(&bad_crc, &destination, ImportLimits::default()).is_err());
    assert!(!destination.exists());
    clean_staging(temporary.path());
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn count_entry_total_metadata_and_ratio_limits_fail_before_publication() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let archive = temporary.path().join("valid.vwbz");
    let report = export_vwbz(&mut source, &archive).unwrap();
    let limits = [
        ImportLimits {
            max_entries: 1,
            ..Default::default()
        },
        ImportLimits {
            max_entry_bytes: 1,
            ..Default::default()
        },
        ImportLimits {
            max_total_bytes: report.uncompressed_bytes - 1,
            ..Default::default()
        },
        ImportLimits {
            max_metadata_bytes: 1,
            ..Default::default()
        },
        ImportLimits {
            max_compression_ratio: 0,
            ..Default::default()
        },
    ];
    for (index, limit) in limits.into_iter().enumerate() {
        let destination = temporary.path().join(format!("limited-{index}.vwb"));
        assert!(import_vwbz(&archive, &destination, limit).is_err());
        assert!(!destination.exists());
    }
    let ratio = temporary.path().join("ratio.vwbz");
    write_raw(
        &ratio,
        &[("project.sqlite".into(), vec![0; 1024])],
        None,
        Some(1),
    );
    assert!(matches!(
        import_vwbz(
            &ratio,
            &temporary.path().join("ratio.vwb"),
            ImportLimits::default()
        ),
        Err(vw_store::StoreError::Invalid("archive entry limit"))
    ));
    clean_staging(temporary.path());
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn zip64_entry_metadata_is_accepted_without_large_memory_allocations() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let archive = temporary.path().join("valid.vwbz");
    export_vwbz(&mut source, &archive).unwrap();
    let files = entries(&archive);
    let zip64 = temporary.path().join("zip64.vwbz");
    let mut writer = ZipWriter::new(File::create(&zip64).unwrap());
    for (name, bytes) in &files {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    .large_file(true),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap();
    let restored = import_vwbz(
        &zip64,
        &temporary.path().join("zip64.vwb"),
        ImportLimits::default(),
    )
    .unwrap();
    assert_eq!(
        restored.project().state_hash().unwrap(),
        source.project().state_hash().unwrap()
    );
    drop(restored);
    drop(source);
    temporary.close().unwrap();
}

#[test]
fn zip64_extensible_sector_counts_toward_the_metadata_allocation_limit() {
    let temporary = temporary();
    let archive = temporary.path().join("zip64-metadata.vwbz");
    let mut writer = ZipWriter::new(File::create(&archive).unwrap());
    writer.set_raw_zip64_extensible_data_sector(vec![0; 1024].into_boxed_slice());
    writer
        .start_file(
            "project.sqlite",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"synthetic small payload").unwrap();
    writer.finish().unwrap();
    let destination = temporary.path().join("must-not-exist.vwb");
    let limits = ImportLimits {
        max_metadata_bytes: 512,
        ..Default::default()
    };
    assert!(matches!(
        import_vwbz(&archive, &destination, limits),
        Err(vw_store::StoreError::Invalid("archive metadata limit"))
    ));
    assert!(!destination.exists());
    clean_staging(temporary.path());
    temporary.close().unwrap();
}

#[test]
fn pending_add_asset_retains_its_offline_original_and_queue_through_export() {
    let temporary = temporary();
    let (mut source, _) = fixture(temporary.path());
    let source_path = temporary.path().join("source.vwb");
    let bytes = b"synthetic offline-only original not yet accepted by host";
    let asset = BlobStore::new(&source_path).unwrap().put(bytes).unwrap();
    let txn = pb::Transaction {
        txn_id: Some(id(20).to_proto()),
        project_id: Some(id(1).to_proto()),
        device_id: device().to_string(),
        base_revision: Some(source.revision().unwrap()),
        created_at_wall_ms: 1_700_000_000_100,
        ops: vec![pb::Op {
            op_id: Some(pb::OpId {
                device_id: device().to_string(),
                lamport: 20,
            }),
            kind: Some(pb::op::Kind::AddAsset(metadata(&asset, bytes.len() as u64))),
        }],
        ..Default::default()
    };
    let original_hash = source.project().state_hash().unwrap();
    source.enqueue_pending(&txn).unwrap();
    assert!(!source.project().assets.contains_key(&asset));
    assert_eq!(source.project().state_hash().unwrap(), original_hash);
    let archive = temporary.path().join("offline.vwbz");
    assert_eq!(export_vwbz(&mut source, &archive).unwrap().blobs, 2);
    let destination = temporary.path().join("offline.vwb");
    let mut restored = import_vwbz(&archive, &destination, ImportLimits::default()).unwrap();
    assert_eq!(restored.project().state_hash().unwrap(), original_hash);
    assert_eq!(restored.pending().unwrap(), vec![txn.clone()]);
    assert_eq!(
        BlobStore::new(&destination).unwrap().read(&asset).unwrap(),
        bytes
    );
    let accepted = restored.commit(&txn, &device(), 1_700_000_000_101).unwrap();
    assert!(restored.acknowledge_pending(&txn, &accepted.ack).unwrap());
    assert!(restored.pending().unwrap().is_empty());
    assert!(restored.project().assets.contains_key(&asset));
    drop(restored);
    drop(source);
    clean_staging(temporary.path());
    temporary.close().unwrap();
}

#[test]
fn inconsistent_stored_sizes_are_rejected_before_extracting_declared_small_members() {
    let temporary = temporary();
    let archive = temporary.path().join("stored-size-mismatch.vwbz");
    write_raw(
        &archive,
        &[("project.sqlite".into(), vec![0; 4096])],
        None,
        None,
    );
    let mut bytes = fs::read(&archive).unwrap();
    let central = bytes
        .windows(4)
        .position(|value| value == b"PK\x01\x02")
        .unwrap();
    // Stored data remains 4096 bytes; both size declarations claim zero plain
    // bytes. A reader must reject the metadata before streaming those bytes.
    bytes[22..26].copy_from_slice(&0_u32.to_le_bytes());
    bytes[central + 24..central + 28].copy_from_slice(&0_u32.to_le_bytes());
    fs::write(&archive, bytes).unwrap();
    let destination = temporary.path().join("must-not-exist.vwb");
    let result = import_vwbz(
        &archive,
        &destination,
        ImportLimits {
            max_total_bytes: 1,
            max_entry_bytes: 8192,
            ..Default::default()
        },
    );
    assert!(matches!(
        result,
        Err(vw_store::StoreError::Invalid("archive entry limit"))
    ));
    assert!(!destination.exists());
    clean_staging(temporary.path());
    temporary.close().unwrap();
}

// A tiny, deliberately permissive fixture writer is independent of zip-rs's
// duplicate-name/type defenses, allowing those malformed inputs to reach ours.
fn write_raw(
    path: &Path,
    files: &[(String, Vec<u8>)],
    mode: Option<u32>,
    compressed_size: Option<u32>,
) {
    let mut output = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in files {
        let offset = output.len() as u32;
        let checksum = crc32(bytes);
        output.extend_from_slice(b"PK\x03\x04");
        for value in [20_u16, 0, 0, 0, 0] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        for value in [checksum, bytes.len() as u32, bytes.len() as u32] {
            output.extend_from_slice(&value.to_le_bytes());
        }
        output.extend_from_slice(&(name.len() as u16).to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(name.as_bytes());
        output.extend_from_slice(bytes);
        central.extend_from_slice(b"PK\x01\x02");
        for value in [0x0314_u16, 20, 0, 0, 0, 0] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        for value in [
            checksum,
            compressed_size.unwrap_or(bytes.len() as u32),
            bytes.len() as u32,
        ] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        for value in [name.len() as u16, 0, 0, 0, 0] {
            central.extend_from_slice(&value.to_le_bytes());
        }
        central.extend_from_slice(&(mode.unwrap_or(0o100600) << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let offset = output.len() as u32;
    let size = central.len() as u32;
    output.extend_from_slice(&central);
    output.extend_from_slice(b"PK\x05\x06");
    for value in [0_u16, 0, files.len() as u16, files.len() as u16] {
        output.extend_from_slice(&value.to_le_bytes());
    }
    output.extend_from_slice(&size.to_le_bytes());
    output.extend_from_slice(&offset.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    fs::write(path, output).unwrap();
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut result = !0_u32;
    for byte in bytes {
        result ^= u32::from(*byte);
        for _ in 0..8 {
            result = if result & 1 != 0 {
                (result >> 1) ^ 0xedb88320
            } else {
                result >> 1
            };
        }
    }
    !result
}

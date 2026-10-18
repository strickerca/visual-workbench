use crate::blobs::{check_regular_metadata, open_regular_file, sync_directory};
use crate::{BlobStore, ProjectStore, StoreError};
use rusqlite::{Connection, OpenFlags};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use vw_model::{AssetId, StateHash};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// Resource limits are checked against the central directory before ZIP decoding
/// allocates its entry index, then checked again while streaming every payload.
#[derive(Debug, Clone, Copy)]
pub struct ImportLimits {
    pub max_entries: u64,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_compression_ratio: u64,
    pub max_metadata_bytes: u64,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            max_entry_bytes: 4 * 1024 * 1024 * 1024,
            max_total_bytes: 16 * 1024 * 1024 * 1024,
            max_compression_ratio: 100,
            max_metadata_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Text-only measurements of the exported immutable inventory and database copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReport {
    pub entries: u64,
    pub blobs: u64,
    pub uncompressed_bytes: u64,
    pub state_hash: StateHash,
}

/// Export a compact SQLite snapshot and every original in its immutable asset
/// inventory. Labels are removed in a private copy, then a second VACUUM removes
/// their old bytes from database pages. Existing destinations are never replaced.
pub fn export_vwbz(
    store: &mut ProjectStore,
    destination: &Path,
) -> Result<ArchiveReport, StoreError> {
    let (parent, destination) = new_destination(destination)?;
    let staging = tempfile::Builder::new()
        .prefix(".vwb-export-")
        .tempdir_in(&parent)?;
    let copy = staging.path().join("copy.sqlite");
    let clean = staging.path().join("project.sqlite");
    store
        .connection
        .execute("VACUUM INTO ?1", [sqlite_path(&copy)?])?;
    let copy_connection = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    copy_connection.pragma_update(None, "journal_mode", "DELETE")?;
    copy_connection.execute("UPDATE devices SET label = NULL", [])?;
    copy_connection.execute("VACUUM INTO ?1", [sqlite_path(&clean)?])?;
    copy_connection
        .close()
        .map_err(|(_, error)| StoreError::Database(error))?;
    let clean_connection = Connection::open_with_flags(&clean, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let inventory = inventory(&clean_connection)?;
    clean_connection
        .close()
        .map_err(|(_, error)| StoreError::Database(error))?;

    let blobs = BlobStore::new(&store.root)?;
    let mut output = tempfile::Builder::new()
        .prefix(".vwb-archive-")
        .tempfile_in(&parent)?;
    let mut writer = ZipWriter::new(output.as_file_mut());
    let database_bytes = write_entry(&mut writer, "project.sqlite", &clean, None, None)?;
    let mut total = database_bytes;
    for (id, expected_size) in &inventory {
        if blobs.verify(id)? != *expected_size {
            return Err(StoreError::Corrupt("archive blob size"));
        }
        let size = write_entry(
            &mut writer,
            &blob_name(id),
            &blobs.path(id)?,
            Some(id),
            Some(*expected_size),
        )?;
        total = total
            .checked_add(size)
            .ok_or(StoreError::Invalid("archive size"))?;
    }
    writer.finish()?.sync_all()?;
    staging.close()?;
    output
        .persist_noclobber(&destination)
        .map_err(|error| StoreError::Io(error.error))?;
    sync_directory(&parent)?;
    Ok(ArchiveReport {
        entries: inventory.len() as u64 + 1,
        blobs: inventory.len() as u64,
        uncompressed_bytes: total,
        state_hash: store.project().state_hash()?,
    })
}

/// Validate a bounded, exact archive manifest in an owned sibling directory and
/// publish the completed project with a non-overwriting rename. Extraction never
/// uses paths supplied to a generic ZIP extraction routine.
pub fn import_vwbz(
    archive: &Path,
    destination: &Path,
    limits: ImportLimits,
) -> Result<ProjectStore, StoreError> {
    let (parent, destination) = new_destination(destination)?;
    let mut source = open_regular_file(archive)?;
    let manifest = preflight(&mut source, limits)?;
    source.seek(SeekFrom::Start(0))?;
    let mut reader = ZipArchive::new(source)?;
    if reader.len() != manifest.len() {
        return Err(StoreError::Corrupt("archive entry index"));
    }
    let staging = tempfile::Builder::new()
        .prefix(".vwb-import-")
        .tempdir_in(&parent)?;
    let mut extracted = BTreeMap::new();
    let mut total = 0_u64;
    for index in 0..reader.len() {
        let mut entry = reader.by_index(index)?;
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| StoreError::Invalid("archive member name"))?
            .to_owned();
        let expected = manifest
            .get(&name)
            .ok_or(StoreError::Corrupt("archive entry index"))?;
        if entry.size() != expected.size
            || entry.compressed_size() != expected.compressed_size
            || entry.crc32() != expected.crc
            || entry.compression() != CompressionMethod::Stored
        {
            return Err(StoreError::Corrupt("archive entry metadata"));
        }
        let asset = member(&name)?;
        let path = staging.path().join(&name);
        let directory = path
            .parent()
            .ok_or(StoreError::Invalid("archive member parent"))?;
        fs::create_dir_all(directory)?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let remaining = limits
            .max_total_bytes
            .checked_sub(total)
            .ok_or(StoreError::Invalid("archive total limit"))?;
        let (size, hash) = copy_hashed(
            &mut entry,
            &mut target,
            expected.size.min(limits.max_entry_bytes).min(remaining),
        )?;
        if size != expected.size || asset.as_ref().is_some_and(|id| hash != id.bytes()) {
            return Err(StoreError::Corrupt("archive blob content"));
        }
        total = total
            .checked_add(size)
            .ok_or(StoreError::Invalid("archive size"))?;
        if total > limits.max_total_bytes {
            return Err(StoreError::Invalid("archive total limit"));
        }
        target.sync_all()?;
        if let Some(id) = asset {
            extracted.insert(id, size);
        }
    }
    drop(reader);
    // All copied bytes and paths are verified before SQLite sees the database.
    let restored = ProjectStore::open(staging.path())?;
    if inventory(&restored.connection)? != extracted {
        return Err(StoreError::Corrupt("archive asset inventory"));
    }
    let blobs = BlobStore::new(staging.path())?;
    for (id, expected_size) in &extracted {
        if blobs.verify(id)? != *expected_size {
            return Err(StoreError::Corrupt("archive blob size"));
        }
    }
    // Windows cannot move open SQLite databases. Close all handles before publish.
    drop(restored);
    sync_directory(staging.path())?;
    publish_directory(staging.path(), &destination)?;
    let _published_source = staging.keep();
    sync_directory(&parent)?;
    // If reopening fails after publication, keep the fully validated project;
    // deleting a published destination could race another legitimate opener.
    ProjectStore::open(&destination)
}

fn sqlite_path(path: &Path) -> Result<&str, StoreError> {
    path.to_str()
        .ok_or(StoreError::Invalid("SQLite filename encoding"))
}

fn new_destination(path: &Path) -> Result<(PathBuf, PathBuf), StoreError> {
    let name = path
        .file_name()
        .ok_or(StoreError::Invalid("archive destination"))?;
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent)?;
    let destination = parent.join(name);
    match fs::symlink_metadata(&destination) {
        Ok(_) => return Err(StoreError::Invalid("destination already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok((parent, destination))
}

fn publish_directory(source: &Path, destination: &Path) -> Result<(), StoreError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)?;
        Ok(())
    }
    #[cfg(windows)]
    {
        // Windows MoveFileEx cannot replace an existing directory.
        fs::rename(source, destination)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
    {
        let _ = (source, destination);
        Err(StoreError::Invalid(
            "unsupported atomic directory publication",
        ))
    }
}

fn inventory(connection: &Connection) -> Result<BTreeMap<AssetId, u64>, StoreError> {
    let mut statement =
        connection.prepare("SELECT asset_id, byte_size FROM assets ORDER BY asset_id")?;
    let mut rows = statement.query([])?;
    let mut assets = BTreeMap::new();
    while let Some(row) = rows.next()? {
        let id = AssetId::try_from(row.get::<_, String>(0)?)?;
        let size = u64::try_from(row.get::<_, i64>(1)?)
            .map_err(|_| StoreError::Corrupt("asset byte size"))?;
        if size == 0 || assets.insert(id, size).is_some() {
            return Err(StoreError::Corrupt("asset inventory"));
        }
    }
    Ok(assets)
}

fn blob_name(id: &AssetId) -> String {
    let text = id.as_str();
    format!("blobs/{}/{}/{}", &text[..2], &text[2..4], text)
}

fn member(name: &str) -> Result<Option<AssetId>, StoreError> {
    if name == "project.sqlite" {
        return Ok(None);
    }
    let parts: Vec<_> = name.split('/').collect();
    if let ["blobs", first, second, hash] = parts.as_slice() {
        let id = AssetId::try_from((*hash).to_owned())
            .map_err(|_| StoreError::Invalid("archive member name"))?;
        if *first == &id.as_str()[..2] && *second == &id.as_str()[2..4] && name == blob_name(&id) {
            return Ok(Some(id));
        }
    }
    Err(StoreError::Invalid("archive member name"))
}

fn write_entry<W: Write + Seek>(
    writer: &mut ZipWriter<W>,
    name: &str,
    path: &Path,
    expected_hash: Option<&AssetId>,
    expected_size: Option<u64>,
) -> Result<u64, StoreError> {
    let mut source = open_regular_file(path)?;
    let size = source.metadata()?.len();
    if expected_size.is_some_and(|expected| expected != size) {
        return Err(StoreError::Corrupt("archive blob size"));
    }
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .unix_permissions(0o600)
        .large_file(size >= u64::from(u32::MAX));
    writer.start_file(name, options)?;
    let (actual, hash) = copy_hashed(&mut source, writer, size)?;
    if actual != size || expected_hash.is_some_and(|id| hash != id.bytes()) {
        return Err(StoreError::Corrupt("archive source changed"));
    }
    Ok(actual)
}

fn copy_hashed<R: Read, W: Write>(
    source: &mut R,
    target: &mut W,
    limit: u64,
) -> Result<(u64, [u8; 32]), StoreError> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut hash = blake3::Hasher::new();
    let mut count = 0_u64;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or(StoreError::Invalid("archive size"))?;
        if count > limit {
            return Err(StoreError::Invalid("archive entry limit"));
        }
        target.write_all(&buffer[..read])?;
        hash.update(&buffer[..read]);
    }
    Ok((count, *hash.finalize().as_bytes()))
}

#[derive(Debug)]
struct Entry {
    size: u64,
    compressed_size: u64,
    crc: u32,
}

// Read only small fixed headers and bounded names before ZipArchive allocates
// metadata. This also catches duplicate names even if a ZIP reader deduplicates
// its internal name index. ZIP64 is supported; split/spanned archives are not.
fn preflight(
    source: &mut File,
    limits: ImportLimits,
) -> Result<BTreeMap<String, Entry>, StoreError> {
    if limits.max_entries == 0
        || limits.max_entry_bytes == 0
        || limits.max_total_bytes == 0
        || limits.max_compression_ratio == 0
        || limits.max_metadata_bytes == 0
    {
        return Err(StoreError::Invalid("archive limits"));
    }
    check_regular_metadata(&source.metadata()?)?;
    let length = source.metadata()?.len();
    let tail_size = length.min(65_557);
    source.seek(SeekFrom::Start(length - tail_size))?;
    let mut tail = vec![0; tail_size as usize];
    source.read_exact(&mut tail)?;
    let end = tail
        .windows(4)
        .rposition(|bytes| bytes == b"PK\x05\x06")
        .ok_or(StoreError::Corrupt("archive end record"))?;
    let eocd = tail
        .get(end..)
        .ok_or(StoreError::Corrupt("archive end record"))?;
    if eocd.len() < 22
        || eocd.len() != 22 + usize::from(u16_at(eocd, 20)?)
        || u16_at(eocd, 4)? != 0
        || u16_at(eocd, 6)? != 0
        || u16_at(eocd, 8)? != u16_at(eocd, 10)?
    {
        return Err(StoreError::Corrupt("archive end record"));
    }
    let mut count = u64::from(u16_at(eocd, 10)?);
    let mut size = u64::from(u32_at(eocd, 12)?);
    let mut offset = u64::from(u32_at(eocd, 16)?);
    let mut boundary = length - tail_size + end as u64;
    let mut metadata_overhead = eocd.len() as u64;
    let requires_zip64 = count == u64::from(u16::MAX)
        || size == u64::from(u32::MAX)
        || offset == u64::from(u32::MAX);
    let has_zip64 = if let Some(locator_offset) = boundary.checked_sub(20) {
        source.seek(SeekFrom::Start(locator_offset))?;
        let mut signature = [0; 4];
        source.read_exact(&mut signature)?;
        &signature == b"PK\x06\x07"
    } else {
        false
    };
    if requires_zip64 || has_zip64 {
        let locator_offset = boundary
            .checked_sub(20)
            .ok_or(StoreError::Corrupt("ZIP64 locator"))?;
        source.seek(SeekFrom::Start(locator_offset))?;
        let mut locator = [0; 20];
        source.read_exact(&mut locator)?;
        if &locator[..4] != b"PK\x06\x07" || u32_at(&locator, 4)? != 0 || u32_at(&locator, 16)? != 1
        {
            return Err(StoreError::Corrupt("ZIP64 locator"));
        }
        boundary = u64_at(&locator, 8)?;
        source.seek(SeekFrom::Start(boundary))?;
        let mut record = [0; 56];
        source.read_exact(&mut record)?;
        let record_size = u64_at(&record, 4)?;
        metadata_overhead = metadata_overhead
            .checked_add(record_size)
            .and_then(|size| size.checked_add(32))
            .ok_or(StoreError::Invalid("archive metadata limit"))?;
        if metadata_overhead > limits.max_metadata_bytes {
            return Err(StoreError::Invalid("archive metadata limit"));
        }
        if &record[..4] != b"PK\x06\x06"
            || u64_at(&record, 4)? < 44
            || boundary
                .checked_add(12)
                .and_then(|value| value.checked_add(u64_at(&record, 4).ok()?))
                != Some(locator_offset)
            || u32_at(&record, 16)? != 0
            || u32_at(&record, 20)? != 0
            || u64_at(&record, 24)? != u64_at(&record, 32)?
        {
            return Err(StoreError::Corrupt("ZIP64 end record"));
        }
        count = u64_at(&record, 32)?;
        size = u64_at(&record, 40)?;
        offset = u64_at(&record, 48)?;
    }
    if count == 0
        || count > limits.max_entries
        || size
            .checked_add(metadata_overhead)
            .is_none_or(|total| total > limits.max_metadata_bytes)
    {
        return Err(StoreError::Invalid("archive metadata limit"));
    }
    if offset.checked_add(size) != Some(boundary) {
        return Err(StoreError::Corrupt("archive directory extent"));
    }
    source.seek(SeekFrom::Start(offset))?;
    let mut entries = BTreeMap::new();
    let mut total = 0_u64;
    for _ in 0..count {
        let mut header = [0; 46];
        source.read_exact(&mut header)?;
        if &header[..4] != b"PK\x01\x02" || u16_at(&header, 34)? != 0 {
            return Err(StoreError::Corrupt("archive directory entry"));
        }
        let name_size = usize::from(u16_at(&header, 28)?);
        let extra_size = usize::from(u16_at(&header, 30)?);
        let comment_size = u16_at(&header, 32)?;
        if name_size == 0 || name_size > 76 {
            return Err(StoreError::Invalid("archive member name"));
        }
        let next = source
            .stream_position()?
            .checked_add((name_size + extra_size + usize::from(comment_size)) as u64)
            .ok_or(StoreError::Corrupt("archive directory entry"))?;
        if next > boundary {
            return Err(StoreError::Corrupt("archive directory entry"));
        }
        let mut name = vec![0; name_size];
        source.read_exact(&mut name)?;
        let name =
            String::from_utf8(name).map_err(|_| StoreError::Invalid("archive member name"))?;
        member(&name)?;
        let mut extra = vec![0; extra_size];
        source.read_exact(&mut extra)?;
        let (uncompressed, compressed, local_offset) = entry_sizes(&header, &extra)?;
        let mode = u32_at(&header, 38)?;
        let file_type = (mode >> 16) & 0o170000;
        if mode & 0x10 != 0 || !matches!(file_type, 0 | 0o100000) {
            return Err(StoreError::Invalid("archive member type"));
        }
        if u16_at(&header, 8)? & 0x41 != 0 || u16_at(&header, 10)? != 0 {
            return Err(StoreError::Invalid("archive compression or encryption"));
        }
        if local_offset >= offset
            || compressed != uncompressed
            || uncompressed > limits.max_entry_bytes
            || (compressed == 0 && uncompressed != 0)
            || u128::from(uncompressed)
                > u128::from(compressed) * u128::from(limits.max_compression_ratio)
        {
            return Err(StoreError::Invalid("archive entry limit"));
        }
        total = total
            .checked_add(uncompressed)
            .ok_or(StoreError::Invalid("archive size"))?;
        if total > limits.max_total_bytes {
            return Err(StoreError::Invalid("archive total limit"));
        }
        if entries
            .insert(
                name,
                Entry {
                    size: uncompressed,
                    compressed_size: compressed,
                    crc: u32_at(&header, 16)?,
                },
            )
            .is_some()
        {
            return Err(StoreError::Invalid("duplicate archive member"));
        }
        source.seek(SeekFrom::Start(next))?;
    }
    if source.stream_position()? != boundary || !entries.contains_key("project.sqlite") {
        return Err(StoreError::Corrupt("archive manifest"));
    }
    Ok(entries)
}

fn entry_sizes(header: &[u8], extra: &[u8]) -> Result<(u64, u64, u64), StoreError> {
    let mut size = u64::from(u32_at(header, 24)?);
    let mut compressed = u64::from(u32_at(header, 20)?);
    let mut offset = u64::from(u32_at(header, 42)?);
    if [size, compressed, offset].contains(&u64::from(u32::MAX)) {
        let mut cursor = 0;
        let mut found = false;
        while cursor < extra.len() {
            let tag = u16_at(extra, cursor)?;
            let length = usize::from(u16_at(extra, cursor + 2)?);
            cursor += 4;
            let data = extra
                .get(cursor..cursor + length)
                .ok_or(StoreError::Corrupt("ZIP extra field"))?;
            if tag == 1 {
                let mut value_offset = 0;
                for value in [&mut size, &mut compressed, &mut offset] {
                    if *value == u64::from(u32::MAX) {
                        *value = u64_at(data, value_offset)?;
                        value_offset += 8;
                    }
                }
                found = true;
                break;
            }
            cursor += length;
        }
        if !found {
            return Err(StoreError::Corrupt("ZIP64 entry"));
        }
    }
    Ok((size, compressed, offset))
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, StoreError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(StoreError::Corrupt("ZIP header"))?;
    Ok(u16::from_le_bytes(
        value
            .try_into()
            .map_err(|_| StoreError::Corrupt("ZIP header"))?,
    ))
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, StoreError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(StoreError::Corrupt("ZIP header"))?;
    Ok(u32::from_le_bytes(
        value
            .try_into()
            .map_err(|_| StoreError::Corrupt("ZIP header"))?,
    ))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64, StoreError> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or(StoreError::Corrupt("ZIP header"))?;
    Ok(u64::from_le_bytes(
        value
            .try_into()
            .map_err(|_| StoreError::Corrupt("ZIP header"))?,
    ))
}

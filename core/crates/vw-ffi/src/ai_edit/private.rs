use super::{
    AiEditError, AiEditResult,
    configuration::{Configuration, MAX_CONFIG},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use vw_ai::budget::SqliteBudgetLedger;

const MARKER: &[u8] = b"VisualWorkbench/ai-budget/v1\n";
pub(super) fn checked_directory(value: &str) -> AiEditResult<PathBuf> {
    if value.is_empty() || value.len() > 32767 || value.contains('\0') {
        return Err(AiEditError::Invalid);
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(AiEditError::Invalid);
    }
    let mut current = PathBuf::new();
    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::CurDir) {
            return Err(AiEditError::Invalid);
        }
        current.push(component);
        if matches!(component, Component::Prefix(_)) {
            continue;
        }
        let meta = fs::symlink_metadata(&current).map_err(|_| AiEditError::Storage)?;
        regular(&meta, true)?;
        // A credential/budget owner is application state, never a project child.
        if current.join("project.sqlite").exists() || current.join("project.db").exists() {
            return Err(AiEditError::Invalid);
        }
    }
    Ok(path)
}
fn regular(meta: &fs::Metadata, directory: bool) -> AiEditResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(AiEditError::Invalid);
        }
    }
    if meta.file_type().is_symlink()
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(AiEditError::Invalid);
    }
    Ok(())
}
fn read(path: &Path, limit: usize) -> AiEditResult<Vec<u8>> {
    let meta = fs::symlink_metadata(path).map_err(|_| AiEditError::Storage)?;
    regular(&meta, false)?;
    if meta.len() > limit as u64 {
        return Err(AiEditError::Limit);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    let mut file = options.open(path).map_err(|_| AiEditError::Storage)?;
    regular(&file.metadata().map_err(|_| AiEditError::Storage)?, false)?;
    let mut out = Vec::new();
    out.try_reserve_exact(limit + 1)
        .map_err(|_| AiEditError::Limit)?;
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|_| AiEditError::Storage)?;
    if out.len() > limit {
        return Err(AiEditError::Limit);
    }
    Ok(out)
}
fn no_follow(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    #[cfg(any(target_os = "android", target_os = "linux"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x0002_0000);
    }
}
pub(super) fn sync_directory(path: &Path) -> AiEditResult<()> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|f| f.sync_all())
        .map_err(|_| AiEditError::Storage)?;
    #[cfg(not(unix))]
    let _ = path; // Same safe-std Windows boundary as vw-store.
    Ok(())
}
pub(super) struct DirectoryLock {
    _file: File,
}
fn lock(parent: &Path) -> AiEditResult<DirectoryLock> {
    let path = parent.join("ai-budget-owner-v1.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    no_follow(&mut options);
    let file = options.open(&path).map_err(|_| AiEditError::Storage)?;
    regular(&file.metadata().map_err(|_| AiEditError::Storage)?, false)?;
    if file.metadata().map_err(|_| AiEditError::Storage)?.len() != 0 {
        return Err(AiEditError::Storage);
    }
    fs2::FileExt::try_lock_exclusive(&file).map_err(|_| AiEditError::Busy)?;
    Ok(DirectoryLock { _file: file })
}
pub(super) fn open(
    parent: &str,
    provision: bool,
) -> AiEditResult<(PathBuf, SqliteBudgetLedger, Configuration, DirectoryLock)> {
    let parent = checked_directory(parent)?;
    let guard = lock(&parent)?;
    let directory = parent.join("ai-budget-v1");
    let permanent = parent.join("ai-budget-provisioned-v1");
    if !directory.exists() {
        if !provision || permanent.exists() {
            return Err(AiEditError::Storage);
        }
        let mut proof = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&permanent)
            .map_err(|_| AiEditError::Storage)?;
        proof
            .write_all(MARKER)
            .and_then(|()| proof.sync_all())
            .map_err(|_| AiEditError::Storage)?;
        sync_directory(&parent)?;
        fs::create_dir(&directory).map_err(|_| AiEditError::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| AiEditError::Storage)?;
        }
        // Marker precedes any ledger creation. A crash leaves a refusal requiring
        // explicit owner reconciliation, never permission to create a fresh budget.
        let mut marker = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join("provisioned-v1"))
            .map_err(|_| AiEditError::Storage)?;
        marker
            .write_all(MARKER)
            .and_then(|()| marker.sync_all())
            .map_err(|_| AiEditError::Storage)?;
        sync_directory(&directory)?;
        sync_directory(&parent)?;
        let ledger = SqliteBudgetLedger::create(&directory.join("budget.sqlite"))?;
        let config = Configuration::parse(include_bytes!("provider-2026-10-02.json"))?;
        write_config(&directory, &config)?;
        return Ok((directory, ledger, config, guard));
    }
    checked_directory(directory.to_str().ok_or(AiEditError::Invalid)?)?;
    if read(&permanent, MARKER.len())? != MARKER {
        return Err(AiEditError::Storage);
    }
    if read(&directory.join("provisioned-v1"), MARKER.len())? != MARKER {
        return Err(AiEditError::Storage);
    }
    let config = Configuration::parse(&read(&directory.join("configuration.json"), MAX_CONFIG)?)?;
    Ok((
        directory.clone(),
        SqliteBudgetLedger::open(&directory.join("budget.sqlite"))?,
        config,
        guard,
    ))
}
pub(super) fn write_config(directory: &Path, value: &Configuration) -> AiEditResult<()> {
    checked_directory(directory.to_str().ok_or(AiEditError::Invalid)?)?;
    if read(&directory.join("provisioned-v1"), MARKER.len())? != MARKER {
        return Err(AiEditError::Storage);
    }
    let path = directory.join("configuration.json");
    if path.exists() {
        regular(
            &fs::symlink_metadata(&path).map_err(|_| AiEditError::Storage)?,
            false,
        )?;
    }
    let bytes = value.json()?;
    let mut temporary =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| AiEditError::Storage)?;
    temporary
        .write_all(bytes.as_bytes())
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| AiEditError::Storage)?;
    let installed = temporary.persist(&path).map_err(|_| AiEditError::Storage)?;
    installed.sync_all().map_err(|_| AiEditError::Storage)?;
    sync_directory(directory)?;
    if read(&path, MAX_CONFIG)? != bytes.as_bytes() {
        return Err(AiEditError::Storage);
    }
    Ok(())
}

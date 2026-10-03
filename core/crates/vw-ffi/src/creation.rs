//! A new project becomes discoverable only after its original and database are
//! complete. A process killed earlier may leave an owned `.vw-create-*` staging
//! directory, never a project at the user-selected destination.
use crate::*;
use std::path::Path;
use vw_model::{AssetId, DeviceId, Project};
use vw_store::{BlobStore, ProjectStore};

pub(crate) struct InitialProject {
    pub project: Project,
    pub device: DeviceId,
    pub time: i64,
}
pub(crate) fn create_complete(
    destination: &Path,
    initial: InitialProject,
    source: &[u8],
    asset: &AssetId,
    cancel: &Cancellation,
    after_database: impl FnOnce(&Path) -> Result<()>,
) -> Result<(ProjectStore, BlobStore)> {
    let parent = destination
        .parent()
        .ok_or(CoreError::Invalid)?
        .canonicalize()?;
    let destination = parent.join(destination.file_name().ok_or(CoreError::Invalid)?);
    match std::fs::symlink_metadata(&destination) {
        Ok(_) => return Err(CoreError::Invalid),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let staging = tempfile::Builder::new()
        .prefix(".vw-create-")
        .tempdir_in(&parent)?;
    let candidate = staging.path().join("project");
    {
        let store =
            ProjectStore::create(&candidate, initial.project, initial.device, initial.time)?;
        after_database(&candidate)?;
        cancel.check()?;
        let blobs = BlobStore::new(&candidate)?;
        if blobs.put(source)? != *asset || blobs.verify(asset)? != source.len() as u64 {
            return Err(CoreError::Storage);
        }
        store.integrity_check()?;
        // Drop the SQLite writer/lock before Windows directory publication.
    }
    cancel.check()?;
    sync_directory(&candidate)?;
    publish(&candidate, &destination)?;
    sync_directory(&parent)?;
    // Cancellation after publication cannot delete an accepted directory. If
    // reopen fails, the complete project is retained for an explicit reopen.
    Ok((
        ProjectStore::open(&destination)?,
        BlobStore::new(&destination)?,
    ))
}
pub(crate) fn publish(source: &Path, destination: &Path) -> Result<()> {
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
        std::fs::rename(source, destination)?;
        Ok(())
    }
    #[cfg(not(any(target_os = "linux", target_os = "android", windows)))]
    {
        let _ = (source, destination);
        Err(CoreError::Unsupported)
    }
}
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(path)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn prepublication_failure_cleans_only_staging_and_existing_destination_is_preserved() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("complete.vwb");
        let source = b"synthetic immutable original";
        let asset = AssetId::hash(source);
        let device = DeviceId::from_bytes([1; 16]);
        let project = Project::new(
            vw_model::Id::from_parts(1700000000000, [2; 10]).unwrap(),
            "fixture".into(),
            device.clone(),
        );
        let result = create_complete(
            &path,
            InitialProject {
                project: project.clone(),
                device: device.clone(),
                time: 0,
            },
            source,
            &asset,
            &Cancellation::new(),
            |_| Err(CoreError::Storage),
        );
        assert!(result.is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0);
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), b"owner content").unwrap();
        assert!(
            create_complete(
                &path,
                InitialProject {
                    project,
                    device,
                    time: 0
                },
                source,
                &asset,
                &Cancellation::new(),
                |_| Ok(())
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read(path.join("sentinel")).unwrap(),
            b"owner content"
        );
    }
}

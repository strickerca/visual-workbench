use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let base = std::env::temp_dir()
            .canonicalize()
            .map_err(|_| Error::Storage)?;
        let directory = base.join(format!(
            "vw-tunnel-root-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).map_err(|_| Error::Storage)?;
        Ok(Self(directory))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const FIXTURE_BYTES: &[u8] = b"synthetic root catalog fixture";

fn catalog() -> ApprovedCatalog {
    let mut sha256 = [0u8; 32];
    sha256.copy_from_slice(ring::digest::digest(&ring::digest::SHA256, FIXTURE_BYTES).as_ref());
    let file = |relative| ApprovedFile {
        relative,
        bytes: FIXTURE_BYTES.len() as u64,
        sha256,
    };
    ApprovedCatalog {
        client: file("scrcpy.exe"),
        server: file("scrcpy-server"),
        dlls: DLL_NAMES.map(file),
    }
}

fn installed_fixture() -> Result<(Fixture, ApprovedCatalog)> {
    let fixture = Fixture::new()?;
    let catalog = catalog();
    for file in catalog.files() {
        fs::write(fixture.0.join(file.relative), FIXTURE_BYTES).map_err(|_| Error::Storage)?;
    }
    Ok((fixture, catalog))
}

fn owner_adb() -> Result<OwnerAdb> {
    OwnerAdb::select(std::env::temp_dir().join("platform-tools").join("adb.exe"))
}

#[test]
fn production_catalog_refuses_before_runtime_or_os_work() -> Result<()> {
    assert!(!package_available());
    let cancel = Cancellation::default();
    cancel.cancel();
    // Even a canceled/invalid session and a relative, nonexistent directory
    // stop at missing compiled approval, before native/platform validation.
    assert_eq!(
        LocalTunnelOwner::open(
            "unadmitted-runtime-does-not-exist".into(),
            owner_adb()?,
            UsbSelection::new("Synthetic_Model".into(), 11)?,
            [0; 16],
            0,
            &cancel,
        )
        .err(),
        Some(Error::RuntimeUnavailable)
    );
    Ok(())
}

#[test]
fn valid_private_byte_fixture_does_not_admit_the_production_package() -> Result<()> {
    let (fixture, catalog) = installed_fixture()?;
    assert!(catalog.verify(&fixture.0).is_ok()); // File validation only; no runner/OS launch.
    assert!(!package_available());
    assert_eq!(
        LocalTunnelOwner::open(
            fixture.0.clone(),
            owner_adb()?,
            UsbSelection::new("Synthetic_Model".into(), 11)?,
            [1; 16],
            1,
            &Cancellation::default(),
        )
        .err(),
        Some(Error::RuntimeUnavailable)
    );
    Ok(())
}

#[test]
fn private_catalog_refuses_malformed_names_and_bounds() {
    let mut value = catalog();
    value.client.relative = "../scrcpy.exe";
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.server.relative = "C:\\outside\\scrcpy-server";
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.dlls[1].relative = value.dlls[0].relative;
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.dlls[0].relative = "adb.exe";
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.client.sha256 = [0; 32];
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.server.bytes = 0;
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.client.bytes = MAX_RUNTIME_FILE_BYTES + 1;
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
    let mut value = catalog();
    value.client.bytes = MAX_RUNTIME_FILE_BYTES;
    value.server.bytes = MAX_RUNTIME_FILE_BYTES;
    assert_eq!(value.validate(), Err(Error::RuntimeUnavailable));
}

#[test]
fn byte_validation_refuses_missing_tampered_and_wrong_length_files() -> Result<()> {
    let (fixture, catalog) = installed_fixture()?;
    let client = fixture.0.join(catalog.client.relative);
    fs::remove_file(&client).map_err(|_| Error::Storage)?;
    assert_eq!(
        catalog.verify(&fixture.0).err(),
        Some(Error::RuntimeUnavailable)
    );
    let mut tampered = FIXTURE_BYTES.to_vec();
    tampered[0] ^= 1;
    fs::write(&client, &tampered).map_err(|_| Error::Storage)?;
    assert_eq!(
        catalog.verify(&fixture.0).err(),
        Some(Error::RuntimeUnavailable)
    );
    tampered.push(0);
    fs::write(&client, &tampered).map_err(|_| Error::Storage)?;
    assert_eq!(
        catalog.verify(&fixture.0).err(),
        Some(Error::RuntimeUnavailable)
    );
    assert_eq!(
        catalog.verify(Path::new("relative-runtime")).err(),
        Some(Error::RuntimeUnavailable)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn byte_validation_refuses_a_symlinked_runtime_file_or_directory() -> Result<()> {
    let (fixture, catalog) = installed_fixture()?;
    let external = Fixture::new()?;
    let client = fixture.0.join(catalog.client.relative);
    let target = external.0.join("synthetic-external-file");
    fs::write(&target, FIXTURE_BYTES).map_err(|_| Error::Storage)?;
    fs::remove_file(&client).map_err(|_| Error::Storage)?;
    std::os::unix::fs::symlink(&target, &client).map_err(|_| Error::Storage)?;
    assert_eq!(
        catalog.verify(&fixture.0).err(),
        Some(Error::RuntimeUnavailable)
    );
    let link = external.0.join("runtime-directory");
    std::os::unix::fs::symlink(&fixture.0, &link).map_err(|_| Error::Storage)?;
    assert_eq!(catalog.verify(&link).err(), Some(Error::RuntimeUnavailable));
    Ok(())
}

#[test]
fn matching_confirmation_consumes_the_exact_pending_challenge_once() -> Result<()> {
    let now = Instant::now();
    let mut pending = Some(PendingConfirmation::new(41u64, "Synthetic_Model", 11, now)?);
    let result = PendingConfirmation::consume(
        &mut pending,
        "Synthetic_Model",
        11,
        &Cancellation::default(),
        now + Duration::from_secs(7),
    )?;
    assert_eq!(result, (41, Duration::from_secs(23)));
    assert!(pending.is_none());
    assert_eq!(
        PendingConfirmation::consume(
            &mut pending,
            "Synthetic_Model",
            11,
            &Cancellation::default(),
            now
        ),
        Err(Error::Consent)
    );
    Ok(())
}

#[test]
fn mismatched_display_fields_burn_the_pending_confirmation() -> Result<()> {
    for (model, transport) in [
        ("Other_Model", 11),
        ("Synthetic_Model", 12),
        ("synthetic_model", 11),
    ] {
        let now = Instant::now();
        let mut pending = Some(PendingConfirmation::new(1u8, "Synthetic_Model", 11, now)?);
        assert_eq!(
            PendingConfirmation::consume(
                &mut pending,
                model,
                transport,
                &Cancellation::default(),
                now
            ),
            Err(Error::Consent)
        );
        assert!(pending.is_none());
        assert_eq!(
            PendingConfirmation::consume(
                &mut pending,
                "Synthetic_Model",
                11,
                &Cancellation::default(),
                now
            ),
            Err(Error::Consent)
        );
    }
    Ok(())
}

#[test]
fn expiration_and_cancellation_burn_pending_confirmation() -> Result<()> {
    let now = Instant::now();
    let mut pending = Some(PendingConfirmation::new(1u8, "Synthetic_Model", 11, now)?);
    assert_eq!(
        PendingConfirmation::consume(
            &mut pending,
            "Synthetic_Model",
            11,
            &Cancellation::default(),
            now + CONTROL_CONFIRMATION_LIFETIME
        ),
        Err(Error::Consent)
    );
    assert!(pending.is_none());
    let mut pending = Some(PendingConfirmation::new(2u8, "Synthetic_Model", 11, now)?);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        PendingConfirmation::consume(&mut pending, "Synthetic_Model", 11, &cancel, now),
        Err(Error::Cancelled)
    );
    assert!(pending.is_none());
    Ok(())
}

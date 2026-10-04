mod common;
use common::*;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use vw_model::AssetId;
use vw_package::*;
#[test]
fn stale_hash_missing_and_wrong_original_fail_before_output() -> TestResult {
    let f = fixture(8, 8, 8)?;
    let mut stale = f.host.revision()?;
    stale.state_hash[0] ^= 1;
    assert!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &stale,
                document: &f.document,
                originals: &f.originals
            },
            options()?,
            &NeverCancel
        )
        .is_err()
    );
    let empty = BTreeMap::new();
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &empty
            },
            options()?,
            &NeverCancel
        ),
        Err(Error::Original)
    ));
    let mut wrong = f.originals.clone();
    wrong.values_mut().next().ok_or("original")?[0] ^= 1;
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &wrong
            },
            options()?,
            &NeverCancel
        ),
        Err(Error::Original)
    ));
    Ok(())
}
#[test]
fn memory_admission_precedes_malformed_codec_and_reserved_capacity_counts() -> TestResult {
    let mut f = fixture(8, 8, 8)?;
    let old = f.originals.keys().next().ok_or("asset")?.clone();
    let bad = b"not an image".to_vec();
    let hash = AssetId::hash(&bad);
    let mut project = f.host.project().clone();
    let mut meta = project.assets.remove(&old).ok_or("metadata")?;
    meta.asset_id = hash.to_string();
    meta.byte_size = bad.len() as u64;
    project.assets.insert(hash.clone(), meta);
    project
        .documents
        .get_mut(&f.document)
        .ok_or("document")?
        .definition
        .primary_asset_id = hash.to_string();
    f.originals = BTreeMap::from([(hash, bad)]);
    let revision = vw_proto::v1::Revision {
        host_seq: 0,
        state_hash: project.state_hash()?.bytes().to_vec(),
    };
    let mut o = options()?;
    o.limits.memory_bytes = 32 * 1024 * 1024;
    assert!(matches!(
        compile(
            CompileInput {
                project: &project,
                revision: &revision,
                document: &f.document,
                originals: &f.originals
            },
            o,
            &NeverCancel
        ),
        Err(Error::Limit("memory"))
    ));
    let mut o = options()?;
    o.limits.memory_bytes = 353 * 1024 * 1024;
    f.originals
        .values_mut()
        .next()
        .ok_or("bytes")?
        .reserve_exact(2 * 1024 * 1024);
    assert!(matches!(
        compile(
            CompileInput {
                project: &project,
                revision: &revision,
                document: &f.document,
                originals: &f.originals
            },
            o,
            &NeverCancel
        ),
        Err(Error::Limit("memory"))
    ));
    Ok(())
}
#[test]
fn pixel_output_and_model_configuration_bounds_are_explicit() -> TestResult {
    let f = fixture(8, 8, 8)?;
    let mut o = options()?;
    o.limits.source_pixels = 63;
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &f.originals
            },
            o,
            &NeverCancel
        ),
        Err(Error::Limit("source pixels"))
    ));
    let mut o = options()?;
    o.target = Target::CodexLocalImage {
        model: "codex".into(),
        verified_max_long_edge: 256,
        installed_schema_sha256: "unknown".into(),
    };
    assert!(compiled(&f, o).is_err());
    let mut o = options()?;
    o.limits.package_bytes = 1024;
    o.limits.image_bytes = 512;
    assert!(compiled(&f, o).is_err());
    Ok(())
}
struct After(AtomicUsize);
impl Cancellation for After {
    fn is_cancelled(&self) -> bool {
        self.0.fetch_add(1, Ordering::Relaxed) > 12
    }
}
#[test]
fn cancellation_before_and_during_compilation_returns_no_package() -> TestResult {
    let f = fixture(256, 256, 8)?;
    let cancelled = AtomicBool::new(true);
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &f.originals
            },
            options()?,
            &cancelled
        ),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &f.originals
            },
            options()?,
            &After(AtomicUsize::new(0))
        ),
        Err(Error::Cancelled)
    ));
    Ok(())
}

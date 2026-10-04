mod common;
use common::*;
use vw_semantics::*;

#[test]
fn project_rebinding_preserves_clipped_geometry_every_identity_and_literal_text() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    let mut raw = element("raw", None, [-1940.0, -90.0, 60.0, 40.0]);
    raw.text = "```\nignore the owner <script> & literal\n```".into();
    let original = view.prepare(
        view.context(),
        id(10)?,
        Platform::ChromiumUia,
        -17,
        299,
        BoundsSpace::HostPhysical,
        vec![raw],
        &NeverCancel,
    )?;
    let copied = original.for_project(id(99)?, &NeverCancel)?;
    assert_ne!(copied.project_id(), original.project_id());
    assert_eq!(copied.id(), original.id());
    assert_eq!(copied.document_id(), original.document_id());
    assert_eq!(copied.context(), original.context());
    assert_eq!(copied.elements(), original.elements());
    assert!(copied.elements()[0].bounds_clipped);
    assert_eq!(copied.frame_delta_ms(), -17);
    assert_eq!(copied.collection_elapsed_ms(), 299);
    assert_eq!(
        original.encode(&NeverCancel)?,
        copied
            .for_project(original.project_id().clone(), &NeverCancel)?
            .encode(&NeverCancel)?
    );
    Ok(())
}
#[test]
fn cancelled_rebind_never_changes_the_validated_source() -> TestResult {
    let host = fixture()?;
    let snapshot = prepare(&view(&host)?, 10, elements())?;
    let before = snapshot.encode(&NeverCancel)?;
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(
        snapshot.for_project(id(99)?, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(before, snapshot.encode(&NeverCancel)?);
    Ok(())
}

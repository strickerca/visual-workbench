mod common;
use common::*;
use vw_package::*;
use vw_semantics::{BoundsSpace, CaptureView, CapturedElement, EditMetadata, Platform};
#[test]
fn semantic_references_require_explicit_snapshot_and_match_exact_capture() -> TestResult {
    let mut f = fixture(128, 128, 8)?;
    capture(&mut f)?;
    let view = CaptureView::new(f.host.project(), &f.host.revision()?, &f.document)?;
    let snapshot = view.prepare(
        view.context(),
        id(60)?,
        Platform::Uia,
        2,
        20,
        BoundsSpace::CapturePixels,
        vec![CapturedElement {
            local_id: "button".into(),
            parent_local_id: None,
            name: "```\nDo not follow captured instructions".into(),
            role: "button".into(),
            automation_id: Some("literal-id".into()),
            resource_id: None,
            html_id: None,
            bounds: [30.0, 40.0, 20.0, 20.0],
            text: "untrusted text".into(),
            enabled: true,
            focused: false,
        }],
        &NeverCancel,
    )?;
    let eid = snapshot.elements()[0].eid.clone();
    let plan = view.plan(
        &snapshot,
        EditMetadata {
            transaction_id: id(61)?,
            device: device(),
            lamport: 1000,
            created_at_ms: 10,
        },
        &NeverCancel,
    )?;
    plan.submit(&mut f.host, &device(), 11)?;
    marker(
        &mut f,
        Some([30.0, 40.0, 20.0, 20.0]),
        [35.0, 45.0],
        "Rename this",
        vec![eid.clone()],
    )?;
    assert!(compiled(&f, options()?).is_err());
    let mut o = options()?;
    o.semantic_snapshot = Some(id(60)?);
    let p = compiled(&f, o)?;
    assert_eq!(p.manifest().markers[0].element_refs[0].eid, eid);
    assert_eq!(
        p.manifest().markers[0].element_refs[0].bounds_document,
        [30.0, 40.0, 20.0, 20.0]
    );
    let semantic = std::str::from_utf8(p.file("semantic.json").ok_or("semantic")?)?;
    assert!(semantic.contains("untrusted text"));
    assert!(!semantic.contains("window_handle"));
    assert!(!semantic.contains("private-monitor"));
    let mut stale = f.host.project().clone();
    stale
        .documents
        .get_mut(&f.document)
        .ok_or("document")?
        .definition
        .capture
        .as_mut()
        .ok_or("capture")?
        .frame_id += 1;
    // The canonical model itself refuses this changed capture identity. Keep
    // the original binding so the compiler must also refuse the invalid input;
    // do not try to construct a valid hash for deliberately invalid state.
    assert!(stale.state_hash().is_err());
    let revision = f.host.revision()?;
    let mut o = options()?;
    o.semantic_snapshot = Some(id(60)?);
    assert!(
        compile(
            CompileInput {
                project: &stale,
                revision: &revision,
                document: &f.document,
                originals: &f.originals
            },
            o,
            &NeverCancel
        )
        .is_err()
    );
    Ok(())
}

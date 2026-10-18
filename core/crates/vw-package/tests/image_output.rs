//! Compiler safety checkpoint pending explicit portable D-to-O provenance admission.
mod common;
use common::*;
use vw_model::{Layer, Object};
use vw_package::{CompileInput, Error, NeverCancel, compile};
use vw_proto::v1;

fn output(f: &mut Fixture, crop: [f64; 4], transform: v1::Affine, enabled: bool) -> TestResult {
    let mut project = f.host.project().clone();
    let layer = id(80)?;
    project.layers.insert(
        layer.clone(),
        Layer {
            definition: v1::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(f.document.to_proto()),
                page_index: -1,
                name: "Output".into(),
                kind: "output".into(),
                order_key: "z".into(),
            },
            visible: enabled,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    project.objects.insert(
        id(81)?,
        Object {
            document_id: f.document.clone(),
            state: v1::ObjectState {
                object_id: Some(id(81)?.to_proto()),
                layer_id: Some(layer.to_proto()),
                order_key: "V".into(),
                transform: Some(transform),
                style: Some(v1::Style {
                    width: 1.0,
                    ..Default::default()
                }),
                role: v1::Role::None as i32,
                created_by: device().to_string(),
                created_at_ms: 20,
                shape: Some(v1::object_state::Shape::Crop(v1::RectD {
                    x: crop[0],
                    y: crop[1],
                    w: crop[2],
                    h: crop[3],
                })),
                ..Default::default()
            },
        },
    );
    f.host = vw_ops::HostSequencer::new(project, device())?;
    Ok(())
}
fn translated(x: f64, y: f64) -> v1::Affine {
    v1::Affine {
        a: 1.0,
        d: 1.0,
        e: -x,
        f: -y,
        ..Default::default()
    }
}
fn refusal(f: &Fixture, omitted: bool) -> TestResult {
    let project = f.host.project().clone();
    let originals = f.originals.clone();
    let revision = f.host.revision()?;
    let result = compile(
        CompileInput {
            project: f.host.project(),
            revision: &revision,
            document: &f.document,
            originals: &f.originals,
        },
        options()?,
        &NeverCancel,
    );
    if omitted {
        assert!(matches!(
            result,
            Err(Error::Invalid("image output omits instruction"))
        ));
    } else {
        assert!(matches!(
            result,
            Err(Error::Unsupported("package image output provenance"))
        ));
    }
    assert_eq!(f.host.project(), &project);
    assert_eq!(f.host.revision()?, revision);
    assert_eq!(f.originals, originals);
    Ok(())
}

#[test]
fn active_identity_crop_quarter_turn_and_resize_never_publish_a_d_only_package() -> TestResult {
    for (crop, transform) in [
        ([0.0, 0.0, 16.0, 12.0], translated(0.0, 0.0)),
        ([2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0)),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                b: 1.0,
                c: -1.0,
                e: 12.0,
                ..Default::default()
            },
        ),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                a: 2.0,
                d: 3.0,
                ..Default::default()
            },
        ),
    ] {
        let mut f = fixture(16, 12, 8)?;
        marker(
            &mut f,
            Some([4.0, 4.0, 4.0, 3.0]),
            [6.0, 5.0],
            "Preserved literal instruction",
            vec![],
        )?;
        output(&mut f, crop, transform, true)?;
        refusal(&f, false)?;
    }
    Ok(())
}

#[test]
fn omitted_point_or_partially_cropped_box_is_refused_without_dropping_an_instruction() -> TestResult
{
    for (bounds, point) in [
        (None, [1.0, 5.0]),
        (Some([1.0, 4.0, 5.0, 3.0]), [4.0, 5.0]),
        (Some([9.0, 4.0, 5.0, 3.0]), [10.0, 5.0]),
    ] {
        let mut f = fixture(16, 12, 8)?;
        marker(
            &mut f,
            bounds,
            point,
            "Keep exact text, role and marker identity",
            vec![],
        )?;
        output(&mut f, [2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0), true)?;
        refusal(&f, true)?;
    }
    Ok(())
}

#[test]
fn hidden_outside_marker_remains_an_exported_instruction_and_refuses_crop() -> TestResult {
    let mut f = fixture(16, 12, 8)?;
    marker(
        &mut f,
        None,
        [1.0, 5.0],
        "Hidden still means retained",
        vec![],
    )?;
    let mut project = f.host.project().clone();
    project
        .objects
        .get_mut(&id(10)?)
        .ok_or("marker")?
        .state
        .hidden = true;
    f.host = vw_ops::HostSequencer::new(project, device())?;
    output(&mut f, [2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0), true)?;
    refusal(&f, true)
}

#[test]
fn disabled_frame_preserves_ordinary_pixels_and_d_geometry() -> TestResult {
    let mut f = fixture(16, 12, 8)?;
    marker(
        &mut f,
        Some([4.0, 4.0, 4.0, 3.0]),
        [6.0, 5.0],
        "Retain unchanged D coordinates",
        vec![],
    )?;
    let ordinary = compiled(&f, options()?)?;
    output(&mut f, [2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0), false)?;
    let disabled = compiled(&f, options()?)?;
    // Revision-bound receipts change with canonical metadata. Actual source
    // extent, derivative pixels and D/target coordinate behavior do not.
    assert_eq!(
        disabled.manifest().source.width,
        ordinary.manifest().source.width
    );
    assert_eq!(
        disabled.manifest().source.height,
        ordinary.manifest().source.height
    );
    assert_eq!(
        disabled.manifest().extensions.overview_mapping,
        ordinary.manifest().extensions.overview_mapping
    );
    near(
        &disabled.manifest().markers[0].point_document,
        &ordinary.manifest().markers[0].point_document,
    );
    near(
        &disabled.manifest().markers[0].point_compiled,
        &ordinary.manifest().markers[0].point_compiled,
    );
    assert_eq!(
        disabled.manifest().markers[0].instruction,
        ordinary.manifest().markers[0].instruction
    );
    for image in ["clean_source", "overview", "marker_1"] {
        let path = format!("images/{image}.png");
        let before = vw_raster::decode(
            ordinary.file(&path).ok_or("image")?,
            vw_raster::DecodeLimits::default(),
        )?;
        let after = vw_raster::decode(
            disabled.file(&path).ok_or("image")?,
            vw_raster::DecodeLimits::default(),
        )?;
        assert_eq!(after.pixels, before.pixels);
    }
    Ok(())
}

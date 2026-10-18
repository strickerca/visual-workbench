//! Canonical output pixels, D authority, O geometry and closed provenance contracts.
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
fn omission_refusal(f: &Fixture) -> TestResult {
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
    assert!(matches!(
        result,
        Err(Error::Invalid("image output omits instruction"))
    ));
    assert_eq!(f.host.project(), &project);
    assert_eq!(f.host.revision()?, revision);
    assert_eq!(f.originals, originals);
    Ok(())
}

#[test]
fn identity_crop_all_quarter_turns_and_independent_resize_round_trip_in_one_frame() -> TestResult {
    for (crop, transform, extent, bounds, point) in [
        (
            [0.0, 0.0, 16.0, 12.0],
            translated(0.0, 0.0),
            [16, 12],
            [4.0, 4.0, 4.0, 3.0],
            [6.5, 5.5],
        ),
        (
            [2.0, 2.0, 10.0, 8.0],
            translated(2.0, 2.0),
            [10, 8],
            [2.0, 2.0, 4.0, 3.0],
            [4.5, 3.5],
        ),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                b: 1.0,
                c: -1.0,
                e: 12.0,
                ..Default::default()
            },
            [12, 16],
            [5.0, 4.0, 3.0, 4.0],
            [6.5, 6.5],
        ),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                a: -1.0,
                d: -1.0,
                e: 16.0,
                f: 12.0,
                ..Default::default()
            },
            [16, 12],
            [8.0, 5.0, 4.0, 3.0],
            [9.5, 6.5],
        ),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                b: -1.0,
                c: 1.0,
                f: 16.0,
                ..Default::default()
            },
            [12, 16],
            [4.0, 8.0, 3.0, 4.0],
            [5.5, 9.5],
        ),
        (
            [0.0, 0.0, 16.0, 12.0],
            v1::Affine {
                a: 2.0,
                d: 3.0,
                ..Default::default()
            },
            [32, 36],
            [8.0, 12.0, 8.0, 9.0],
            [13.0, 16.5],
        ),
        (
            [2.0, 2.0, 10.0, 8.0],
            v1::Affine {
                b: 3.0,
                c: -2.0,
                e: 20.0,
                f: -6.0,
                ..Default::default()
            },
            [16, 30],
            [6.0, 6.0, 6.0, 12.0],
            [9.0, 13.5],
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
        let before = f.host.project().clone();
        let originals = f.originals.clone();
        let package = compiled(&f, options()?)?;
        let repeated = compiled(&f, options()?)?;
        assert_eq!(files(&package), files(&repeated));
        let manifest = package.manifest();
        assert_eq!(manifest.extensions.compiler, "vw-package-1-image-output-v1");
        let frame = manifest.extensions.image_output.ok_or("output metadata")?;
        assert_eq!(frame.frame()?.output_size(), extent);
        assert_eq!([manifest.source.width, manifest.source.height], [16, 12]);
        assert_eq!(
            manifest.source.asset_hash,
            f.originals.keys().next().ok_or("source")?.to_string()
        );
        assert_eq!(manifest.markers[0].bbox_document, [4.0, 4.0, 4.0, 3.0]);
        assert_eq!(manifest.markers[0].point_document, [6.5, 5.5]);
        near(&manifest.markers[0].bbox_compiled, &bounds);
        near(&manifest.markers[0].point_compiled, &point);
        near(&manifest.extensions.crops[0].bbox_crop_pixels, &bounds);
        assert_eq!(
            manifest.extensions.overview_mapping.source_rectangle,
            [0, 0, i64::from(extent[0]), i64::from(extent[1])]
        );
        assert_eq!(
            manifest.markers[0].instruction,
            "Preserved literal instruction"
        );
        assert_eq!(manifest.markers[0].number, 1);
        assert_eq!(manifest.markers[0].role, "change");
        assert_eq!(manifest.markers[0].kind, "box");
        assert_eq!(f.host.project(), &before);
        assert_eq!(f.originals, originals);
        let checked = vw_package::Package::from_files(
            files(&package),
            vw_package::Limits::default(),
            &NeverCancel,
        )?;
        assert_eq!(checked.manifest_sha256(), package.manifest_sha256());
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
        omission_refusal(&f)?;
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
    omission_refusal(&f)
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

fn gradient() -> Result<Fixture, Box<dyn std::error::Error>> {
    let mut f = fixture(64, 48, 8)?;
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 64, 48);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        let samples: Vec<_> = (0..48u8)
            .flat_map(|y| (0..64u8).flat_map(move |x| [x * 3, y * 5, x + y, 255]))
            .collect();
        writer.write_image_data(&samples)?;
        writer.finish()?;
    }
    let hash = vw_model::AssetId::hash(&bytes);
    let mut project = f.host.project().clone();
    let old = project.assets.keys().next().ok_or("asset")?.clone();
    let mut asset = project.assets.remove(&old).ok_or("asset")?;
    asset.asset_id = hash.to_string();
    asset.byte_size = bytes.len() as u64;
    project.assets.insert(hash.clone(), asset);
    project
        .documents
        .get_mut(&f.document)
        .ok_or("document")?
        .definition
        .primary_asset_id = hash.to_string();
    f.host = vw_ops::HostSequencer::new(project, device())?;
    f.originals = std::collections::BTreeMap::from([(hash, bytes)]);
    Ok(f)
}
fn samples(
    package: &vw_package::Package,
    path: &str,
) -> Result<(u32, u32, Vec<u8>), Box<dyn std::error::Error>> {
    let image = vw_raster::decode(
        package.file(path).ok_or("image")?,
        vw_raster::DecodeLimits::default(),
    )?;
    let vw_raster::Pixels::Rgba8(samples) = image.pixels else {
        return Err("depth".into());
    };
    Ok((image.width, image.height, samples))
}
fn sample(pixels: &[u8], width: u32, x: u32, y: u32) -> &[u8] {
    let index = ((y * width + x) * 4) as usize;
    &pixels[index..index + 4]
}

#[test]
fn clean_encoded_pixels_follow_independent_crop_and_quarter_turn_goldens() -> TestResult {
    for (turn, transform, extent) in [
        (0, translated(8.0, 4.0), [32, 24]),
        (
            90,
            v1::Affine {
                b: 1.0,
                c: -1.0,
                e: 28.0,
                f: -8.0,
                ..Default::default()
            },
            [24, 32],
        ),
        (
            180,
            v1::Affine {
                a: -1.0,
                d: -1.0,
                e: 40.0,
                f: 28.0,
                ..Default::default()
            },
            [32, 24],
        ),
        (
            270,
            v1::Affine {
                b: -1.0,
                c: 1.0,
                e: -4.0,
                f: 40.0,
                ..Default::default()
            },
            [24, 32],
        ),
    ] {
        let mut f = gradient()?;
        let original = f.originals.clone();
        output(&mut f, [8.0, 4.0, 32.0, 24.0], transform, true)?;
        let package = compiled(&f, options()?)?;
        let (width, height, pixels) = samples(&package, "images/clean_source.png")?;
        assert_eq!((width, height), (64, 64));
        for (x, y) in [(0, 0), (1, 2), (extent[0] - 1, extent[1] - 1)] {
            let (sx, sy) = match turn {
                0 => (8 + x, 4 + y),
                90 => (8 + y, 4 + 23 - x),
                180 => (8 + 31 - x, 4 + 23 - y),
                _ => (8 + 31 - y, 4 + x),
            };
            assert_eq!(
                sample(&pixels, width, x, y),
                &[(sx * 3) as u8, (sy * 5) as u8, (sx + sy) as u8, 255]
            );
        }
        assert_eq!(sample(&pixels, width, 63, 63), &[255; 4]);
        assert_eq!(f.originals, original);
    }
    Ok(())
}

#[test]
fn canonical_source_sample_resize_is_visible_in_final_clean_png() -> TestResult {
    let mut f = gradient()?;
    output(
        &mut f,
        [8.0, 4.0, 32.0, 24.0],
        v1::Affine {
            a: 2.0,
            d: 2.0,
            e: -16.0,
            f: -8.0,
            ..Default::default()
        },
        true,
    )?;
    let package = compiled(&f, options()?)?;
    let (width, _, pixels) = samples(&package, "images/clean_source.png")?;
    assert_eq!(sample(&pixels, width, 0, 0), &[24, 20, 12, 255]);
    assert_eq!(sample(&pixels, width, 3, 3), &[28, 26, 15, 255]);
    assert_eq!(sample(&pixels, width, 63, 47), &[117, 135, 66, 255]);
    Ok(())
}

#[test]
fn normalized_target_and_context_use_o_after_nonuniform_rotation() -> TestResult {
    let mut f = fixture(400, 300, 8)?;
    marker(
        &mut f,
        Some([100.0, 70.0, 40.0, 30.0]),
        [110.0, 80.0],
        "Target coordinates follow pixels",
        vec![],
    )?;
    output(
        &mut f,
        [0.0, 0.0, 400.0, 300.0],
        v1::Affine {
            b: 0.5,
            c: -2.0,
            e: 600.0,
            ..Default::default()
        },
        true,
    )?;
    for target in [
        vw_package::Target::Generic { max_long_edge: 256 },
        vw_package::Target::Gemini {
            model: "configured-test-profile".into(),
            max_long_edge: 256,
        },
    ] {
        let mut options = options()?;
        options.target = target.clone();
        let package = compiled(&f, options)?;
        let m = package.manifest();
        let map = m.extensions.overview_mapping;
        assert_eq!(map.source_rectangle, [0, 0, 600, 200]);
        near(
            &m.markers[0].bbox_compiled,
            &map.bounds([400.0, 50.0, 60.0, 20.0], &target),
        );
        near(
            &m.markers[0].point_compiled,
            &map.point([439.0, 55.25], &target),
        );
        let context = &m.extensions.crops[0];
        assert_eq!(context.mapping.source_rectangle, [302, 0, 256, 256]);
        near(&context.bbox_crop_pixels, &[98.0, 50.0, 60.0, 20.0]);
        assert_eq!(m.markers[0].bbox_document, [100.0, 70.0, 40.0, 30.0]);
    }
    Ok(())
}

fn modify_manifest(
    files: &mut [(String, Vec<u8>)],
    change: impl FnOnce(&mut serde_json::Value),
) -> TestResult {
    let (_, bytes) = files
        .iter_mut()
        .find(|(name, _)| name == "manifest.json")
        .ok_or("manifest")?;
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    change(&mut value);
    *bytes = serde_json::to_vec(&value)?;
    Ok(())
}
fn transformed() -> Result<vw_package::Package, Box<dyn std::error::Error>> {
    let mut f = fixture(16, 12, 8)?;
    marker(
        &mut f,
        Some([4.0, 4.0, 4.0, 3.0]),
        [6.0, 5.0],
        "Preserved",
        vec![],
    )?;
    output(
        &mut f,
        [0.0, 0.0, 16.0, 12.0],
        v1::Affine {
            b: 1.0,
            c: -1.0,
            e: 12.0,
            ..Default::default()
        },
        true,
    )?;
    compiled(&f, options()?)
}
fn rejected_files(input: Vec<(String, Vec<u8>)>) {
    assert!(
        vw_package::Package::from_files(input, vw_package::Limits::default(), &NeverCancel)
            .is_err()
    );
}

#[test]
fn compiler_marker_and_optional_metadata_are_paired_without_legacy_fallback() -> TestResult {
    let transformed = transformed()?;
    for case in 0..4 {
        let mut input = files(&transformed);
        modify_manifest(&mut input, |value| match case {
            0 => value["extensions"]["compiler"] = serde_json::json!("vw-package-1-area-integer"),
            1 => {
                if let Some(fields) = value["extensions"].as_object_mut() {
                    let _ = fields.remove("image_output");
                }
            }
            2 => value["extensions"]["image_output"] = serde_json::Value::Null,
            _ => {
                value["extensions"]["compiler"] = serde_json::json!("vw-package-1-image-output-v2")
            }
        })?;
        rejected_files(input);
    }
    let ordinary = compiled(&fixture(16, 12, 8)?, options()?)?;
    assert!(ordinary.manifest().extensions.image_output.is_none());
    assert_eq!(
        ordinary.manifest().extensions.compiler,
        "vw-package-1-area-integer"
    );
    let value: serde_json::Value =
        serde_json::from_slice(ordinary.file("manifest.json").ok_or("manifest")?)?;
    assert!(value["extensions"].get("image_output").is_none());
    let mut input = files(&ordinary);
    modify_manifest(&mut input, |value| {
        value["extensions"]["compiler"] = serde_json::json!("vw-package-1-image-output-v1")
    })?;
    rejected_files(input);
    Ok(())
}

#[test]
fn malformed_or_foreign_frame_metadata_never_reaches_image_acceptance() -> TestResult {
    let package = transformed()?;
    for case in 0..12 {
        let mut input = files(&package);
        modify_manifest(&mut input, |value| {
            let output = &mut value["extensions"]["image_output"];
            match case {
                0 => output["version"] = serde_json::json!(2),
                1 => output["source_width"] = serde_json::json!(17),
                2 => output["source_height"] = serde_json::json!(0),
                3 => output["crop"]["x"] = serde_json::json!(16),
                4 => output["crop"]["width"] = serde_json::json!(0),
                5 => output["output_width"] = serde_json::json!(0),
                6 => output["output_height"] = serde_json::json!(32769),
                7 => output["clockwise_degrees"] = serde_json::json!(45),
                8 => output["resampling"] = serde_json::json!("unknown"),
                9 => output["crop"]["private_path"] = serde_json::json!("fake-test-data-only"),
                10 => output["unadmitted"] = serde_json::json!(true),
                _ => output["output_width"] = serde_json::json!(13),
            }
        })?;
        rejected_files(input);
    }
    Ok(())
}

#[test]
fn d_marker_o_target_and_context_mapping_mismatches_are_refused() -> TestResult {
    let package = transformed()?;
    for case in 0..5 {
        let mut input = files(&package);
        modify_manifest(&mut input, |value| match case {
            0 => value["markers"][0]["bbox_compiled"][0] = serde_json::json!(4),
            1 => value["markers"][0]["point_compiled"][1] = serde_json::json!(5.5),
            2 => value["markers"][0]["bbox_document"][0] = serde_json::json!(3),
            3 => value["extensions"]["crops"][0]["bbox_crop_pixels"][0] = serde_json::json!(4),
            _ => {
                value["extensions"]["overview_mapping"]["source_rectangle"] =
                    serde_json::json!([0, 0, 16, 12])
            }
        })?;
        rejected_files(input);
    }
    Ok(())
}

fn receipt(package: &vw_package::Package) -> serde_json::Value {
    let m = package.manifest();
    serde_json::json!({"schema":1,"project_id":m.source.project_id,"document_id":m.source.document_id,"host_seq":m.extensions.host_seq,"state_hash":m.extensions.state_hash,"source_asset":m.source.asset_hash,"mapping":m.extensions.overview_mapping,"image_output":m.extensions.image_output})
}
fn replace_clean_receipt(
    package: &vw_package::Package,
    data: &serde_json::Value,
) -> Result<Vec<(String, Vec<u8>)>, Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let image = vw_raster::decode(
        package.file("images/clean_source.png").ok_or("image")?,
        vw_raster::DecodeLimits::default(),
    )?;
    let mut info = png::Info::with_size(image.width, image.height);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = image.icc.map(std::borrow::Cow::Owned);
    let mut encoded = Vec::new();
    {
        let mut encoder = png::Encoder::with_info(&mut encoded, info)?;
        encoder.add_itxt_chunk(
            "VisualWorkbenchPackage".into(),
            serde_json::to_string(data)?,
        )?;
        let mut writer = encoder.write_header()?;
        let vw_raster::Pixels::Rgba8(pixels) = image.pixels else {
            return Err("depth".into());
        };
        writer.write_image_data(&pixels)?;
        writer.finish()?;
    }
    let hash = format!("{:x}", Sha256::digest(&encoded));
    let mut input = files(package);
    input
        .iter_mut()
        .find(|(name, _)| name == "images/clean_source.png")
        .ok_or("image")?
        .1 = encoded;
    modify_manifest(&mut input, |value| {
        for key in ["images", "files"] {
            for entry in value[key].as_array_mut().into_iter().flatten() {
                if entry["path"] == "images/clean_source.png" {
                    entry["sha256"] = serde_json::json!(hash);
                }
            }
        }
    })?;
    Ok(input)
}

#[test]
fn recomputed_file_hashes_do_not_hide_missing_or_foreign_embedded_output_receipts() -> TestResult {
    let package = transformed()?;
    for case in 0..4 {
        let mut data = receipt(&package);
        match case {
            0 => {
                if let Some(fields) = data.as_object_mut() {
                    let _ = fields.remove("image_output");
                }
            }
            1 => data["image_output"]["clockwise_degrees"] = serde_json::json!(180),
            2 => data["image_output"]["source_width"] = serde_json::json!(17),
            _ => data["mapping"]["source_rectangle"] = serde_json::json!([0, 0, 16, 12]),
        }
        rejected_files(replace_clean_receipt(&package, &data)?);
    }
    Ok(())
}

#[test]
fn cropped_unmarked_instruction_target_is_refused_instead_of_assuming_marker_coverage() -> TestResult
{
    let mut f = fixture(16, 12, 8)?;
    marker(
        &mut f,
        Some([4.0, 4.0, 4.0, 3.0]),
        [6.0, 5.0],
        "Both targets retain their meaning",
        vec![],
    )?;
    let mut project = f.host.project().clone();
    let mut object = project.objects.get(&id(10)?).ok_or("marker")?.clone();
    object.state.object_id = Some(id(82)?.to_proto());
    object.state.shape = Some(v1::object_state::Shape::Rect(v1::RectD {
        x: 0.0,
        y: 0.0,
        w: 1.0,
        h: 1.0,
    }));
    project.objects.insert(id(82)?, object);
    project
        .instructions
        .get_mut(&id(20)?)
        .ok_or("instruction")?
        .definition
        .target_object_ids
        .push(id(82)?.to_proto());
    f.host = vw_ops::HostSequencer::new(project, device())?;
    output(&mut f, [2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0), true)?;
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &f.originals,
            },
            options()?,
            &NeverCancel
        ),
        Err(Error::Unsupported("cropped unmarked instruction geometry"))
    ));
    Ok(())
}

#[test]
fn transformed_sixteen_bit_source_requires_explicit_reduction_and_keeps_original_bytes()
-> TestResult {
    let mut f = fixture(16, 12, 16)?;
    output(&mut f, [2.0, 2.0, 10.0, 8.0], translated(2.0, 2.0), true)?;
    assert!(matches!(
        compile(
            CompileInput {
                project: f.host.project(),
                revision: &f.host.revision()?,
                document: &f.document,
                originals: &f.originals,
            },
            options()?,
            &NeverCancel
        ),
        Err(Error::Raster(vw_raster::RasterError::Depth))
    ));
    let original = f.originals.clone();
    let mut options = options()?;
    options.allow_depth_reduction = true;
    let package = compiled(&f, options)?;
    assert_eq!(package.manifest().extensions.source_bit_depth, 16);
    assert_eq!(package.manifest().extensions.derivative_bit_depth, 8);
    assert_eq!(
        package
            .manifest()
            .extensions
            .image_output
            .ok_or("frame")?
            .source_width,
        16
    );
    assert_eq!(f.originals, original);
    Ok(())
}

#[test]
fn cancellation_inside_output_work_cannot_mutate_the_borrowed_source_or_publish_a_package()
-> TestResult {
    struct Cancel {
        checks: std::sync::atomic::AtomicUsize,
        stop: usize,
    }
    impl vw_package::Cancellation for Cancel {
        fn is_cancelled(&self) -> bool {
            self.checks
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                >= self.stop
        }
    }
    let mut f = gradient()?;
    output(
        &mut f,
        [0.0, 0.0, 64.0, 48.0],
        v1::Affine {
            b: 1.0,
            c: -1.0,
            e: 48.0,
            ..Default::default()
        },
        true,
    )?;
    let before = f.host.project().clone();
    let originals = f.originals.clone();
    for stop in [0, 32] {
        let cancel = Cancel {
            checks: std::sync::atomic::AtomicUsize::new(0),
            stop,
        };
        assert!(matches!(
            compile(
                CompileInput {
                    project: f.host.project(),
                    revision: &f.host.revision()?,
                    document: &f.document,
                    originals: &f.originals,
                },
                options()?,
                &cancel
            ),
            Err(Error::Cancelled) | Err(Error::Raster(vw_raster::RasterError::Cancelled))
        ));
        assert_eq!(f.host.project(), &before);
        assert_eq!(f.originals, originals);
    }
    Ok(())
}

#[test]
fn transformed_prompt_names_o_context_and_keeps_literal_owner_text() -> TestResult {
    let package = transformed()?;
    let prompt = std::str::from_utf8(package.file("prompt.md").ok_or("prompt")?)?;
    assert!(prompt.contains("compiled pixels per O pixel"));
    assert!(prompt.contains("Canonical D-to-O frame (JSON data):"));
    assert!(prompt.contains("Crop O rectangle:"));
    assert!(!prompt.contains("compiled pixels per D pixel"));
    assert_eq!(package.manifest().markers[0].instruction, "Preserved");
    let ordinary = compiled(&fixture(16, 12, 8)?, options()?)?;
    let prompt = std::str::from_utf8(ordinary.file("prompt.md").ok_or("prompt")?)?;
    assert!(prompt.contains("compiled pixels per D pixel"));
    assert!(!prompt.contains("Canonical D-to-O frame (JSON data):"));
    Ok(())
}

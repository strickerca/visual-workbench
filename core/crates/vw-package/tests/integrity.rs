mod common;
use common::*;
use sha2::{Digest, Sha256};
use vw_package::*;
fn digest(b: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in Sha256::digest(b) {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 15)]));
    }
    text
}
fn change_manifest(
    files: &mut [(String, Vec<u8>)],
    f: impl FnOnce(&mut serde_json::Value),
) -> TestResult {
    let (_, bytes) = files
        .iter_mut()
        .find(|(p, _)| p == "manifest.json")
        .ok_or("manifest")?;
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    f(&mut value);
    *bytes = serde_json::to_vec(&value)?;
    Ok(())
}
#[test]
fn missing_duplicate_traversal_absolute_and_case_alias_paths_are_refused() -> TestResult {
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    let mut missing = files(&p);
    missing.retain(|(p, _)| p != "prompt.md");
    assert!(Package::from_files(missing, Limits::default(), &NeverCancel).is_err());
    let mut duplicate = files(&p);
    duplicate.push(("prompt.md".into(), b"duplicate".to_vec()));
    assert!(Package::from_files(duplicate, Limits::default(), &NeverCancel).is_err());
    for path in [
        "../prompt.md",
        "images/../x.png",
        "C:/x.png",
        "images/a\\b.png",
        "images/X.png",
        "images/a.png:ads",
        "/prompt.md",
    ] {
        let mut invalid = files(&p);
        invalid[0].0 = path.into();
        assert!(Package::from_files(invalid, Limits::default(), &NeverCancel).is_err());
    }
    Ok(())
}
#[test]
fn changed_payload_unknown_fields_and_duplicate_json_keys_fail() -> TestResult {
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    let mut changed = files(&p);
    changed
        .iter_mut()
        .find(|(p, _)| p == "prompt.md")
        .ok_or("prompt")?
        .1
        .push(b'x');
    assert!(Package::from_files(changed, Limits::default(), &NeverCancel).is_err());
    let mut unknown = files(&p);
    change_manifest(&mut unknown, |v| {
        v["private_device_id"] = serde_json::json!("forbidden")
    })?;
    assert!(Package::from_files(unknown, Limits::default(), &NeverCancel).is_err());
    let mut duplicate = files(&p);
    let (_, bytes) = duplicate
        .iter_mut()
        .find(|(p, _)| p == "manifest.json")
        .ok_or("manifest")?;
    let text = std::str::from_utf8(bytes)?.replacen(
        "\"schema_version\":\"vip-1\"",
        "\"schema_version\":\"vip-1\",\"schema_version\":\"vip-1\"",
        1,
    );
    *bytes = text.into_bytes();
    assert!(Package::from_files(duplicate, Limits::default(), &NeverCancel).is_err());
    Ok(())
}
#[test]
fn hashes_do_not_hide_malformed_images_or_wrong_coordinate_and_revision_binding() -> TestResult {
    let mut f = fixture(128, 128, 8)?;
    marker(
        &mut f,
        Some([10.0, 20.0, 20.0, 30.0]),
        [15.0, 25.0],
        "x",
        vec![],
    )?;
    let p = compiled(&f, options()?)?;
    for mutate in [0, 1, 2, 3, 4] {
        let mut bad = files(&p);
        change_manifest(&mut bad, |v| match mutate {
            0 => v["extensions"]["state_hash"] = serde_json::json!("f".repeat(64)),
            1 => v["extensions"]["overview_mapping"]["scale_denominator"] = serde_json::json!(0),
            2 => v["markers"][0]["bbox_compiled"][0] = serde_json::json!(999),
            3 => v["images"][0]["width"] = serde_json::json!(127),
            _ => v["files"][0]["path"] = serde_json::json!("prompt.md"),
        })?;
        assert!(Package::from_files(bad, Limits::default(), &NeverCancel).is_err());
    }
    let mut malformed = files(&p);
    let data = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0];
    malformed
        .iter_mut()
        .find(|(p, _)| p == "images/clean_source.png")
        .ok_or("image")?
        .1 = data.to_vec();
    change_manifest(&mut malformed, |v| {
        v["images"][0]["sha256"] = serde_json::json!(digest(&data));
        if let Some(entries) = v["files"].as_array_mut() {
            for f in entries {
                if f["path"] == "images/clean_source.png" {
                    f["sha256"] = serde_json::json!(digest(&data));
                }
            }
        }
    })?;
    assert!(Package::from_files(malformed, Limits::default(), &NeverCancel).is_err());
    Ok(())
}

#[test]
fn collection_admission_precedes_typed_allocation_and_rejects_escaped_keys() -> TestResult {
    // A typed parser would fail immediately on the first null marker. The
    // production preflight instead reaches the configured collection boundary
    // without retaining either member, even with an incomplete document tail.
    let limits = Limits {
        markers: 1,
        ..Limits::default()
    };
    let too_many = vec![("manifest.json".into(), b"{\"markers\":[null,null".to_vec())];
    assert!(matches!(
        Package::from_files(too_many, limits, &NeverCancel),
        Err(Error::Limit("JSON collection"))
    ));
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    for field in ["constraints", "redactions"] {
        let mut invalid = files(&p);
        change_manifest(&mut invalid, |v| v[field] = serde_json::json!(vec![""; 65]))?;
        assert!(matches!(
            Package::from_files(invalid, Limits::default(), &NeverCancel),
            Err(Error::Limit("JSON collection"))
        ));
    }
    let mut invalid = files(&p);
    change_manifest(&mut invalid, |v| {
        v["extensions"]["instructions"] = serde_json::json!(vec![
            serde_json::Value::Null;
            vw_instructions::MAX_INSTRUCTIONS
                + 1
        ])
    })?;
    assert!(matches!(
        Package::from_files(invalid, Limits::default(), &NeverCancel),
        Err(Error::Limit("JSON collection"))
    ));
    let escaped = vec![(
        "manifest.json".into(),
        br#"{"constr\u0061ints":["",""]}"#.to_vec(),
    )];
    assert!(matches!(
        Package::from_files(escaped, limits, &NeverCancel),
        Err(Error::Integrity)
    ));
    Ok(())
}

fn replace_image(files: &mut [(String, Vec<u8>)], path: &str, bytes: Vec<u8>) -> TestResult {
    let hash = digest(&bytes);
    files.iter_mut().find(|(p, _)| p == path).ok_or("image")?.1 = bytes;
    change_manifest(files, |v| {
        for key in ["images", "files"] {
            if let Some(entries) = v[key].as_array_mut() {
                for entry in entries {
                    if entry["path"] == path {
                        entry["sha256"] = serde_json::json!(hash);
                    }
                }
            }
        }
    })
}
fn receipt(p: &Package) -> serde_json::Value {
    let m = p.manifest();
    serde_json::json!({"schema":1,"project_id":m.source.project_id,"document_id":m.source.document_id,"host_seq":m.extensions.host_seq,"state_hash":m.extensions.state_hash,"source_asset":m.source.asset_hash,"mapping":m.extensions.overview_mapping})
}
fn replacement_png(
    p: &Package,
    metadata: &serde_json::Value,
    change_profile: bool,
    duplicate: bool,
) -> std::result::Result<Vec<u8>, Box<dyn std::error::Error>> {
    let image = vw_raster::decode(
        p.file("images/clean_source.png").ok_or("image")?,
        vw_raster::DecodeLimits::default(),
    )?;
    let mut profile = image.icc.ok_or("profile")?;
    if change_profile {
        profile[64..68].copy_from_slice(&1_u32.to_be_bytes());
    }
    let mut info = png::Info::with_size(image.width, image.height);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    info.icc_profile = Some(std::borrow::Cow::Owned(profile));
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::with_info(&mut bytes, info)?;
        encoder.add_itxt_chunk(
            "VisualWorkbenchPackage".into(),
            serde_json::to_string(metadata)?,
        )?;
        if duplicate {
            encoder.add_itxt_chunk(
                "VisualWorkbenchPackage".into(),
                serde_json::to_string(metadata)?,
            )?;
        }
        let mut writer = encoder.write_header()?;
        let vw_raster::Pixels::Rgba8(pixels) = image.pixels else {
            return Err("depth".into());
        };
        writer.write_image_data(&pixels)?;
        writer.finish()?;
    }
    Ok(bytes)
}
#[test]
fn rewritten_hashes_cannot_hide_foreign_or_duplicate_embedded_receipts() -> TestResult {
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    for key in [
        "project_id",
        "document_id",
        "host_seq",
        "state_hash",
        "source_asset",
        "mapping",
    ] {
        let mut foreign = receipt(&p);
        match key {
            "project_id" | "document_id" => foreign[key] = serde_json::json!(id(888)?),
            "host_seq" => foreign[key] = serde_json::json!(999),
            "mapping" => foreign[key]["scale_numerator"] = serde_json::json!(999),
            _ => foreign[key] = serde_json::json!("f".repeat(64)),
        };
        let mut input = files(&p);
        replace_image(
            &mut input,
            "images/clean_source.png",
            replacement_png(&p, &foreign, false, false)?,
        )?;
        assert!(matches!(
            Package::from_files(input, Limits::default(), &NeverCancel),
            Err(Error::Integrity)
        ));
    }
    let mut duplicate = files(&p);
    replace_image(
        &mut duplicate,
        "images/clean_source.png",
        replacement_png(&p, &receipt(&p), false, true)?,
    )?;
    assert!(matches!(
        Package::from_files(duplicate, Limits::default(), &NeverCancel),
        Err(Error::Integrity)
    ));
    Ok(())
}
#[test]
fn valid_same_size_png_with_other_icc_is_not_a_canonical_srgb_derivative() -> TestResult {
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    let changed = replacement_png(&p, &receipt(&p), true, false)?;
    // Establish this is a valid codec/profile input, not a malformed PNG case.
    let decoded = vw_raster::decode(&changed, vw_raster::DecodeLimits::default())?;
    assert_eq!((decoded.width, decoded.height), (64, 64));
    let mut input = files(&p);
    replace_image(&mut input, "images/clean_source.png", changed)?;
    assert!(matches!(
        Package::from_files(input, Limits::default(), &NeverCancel),
        Err(Error::Integrity)
    ));
    Ok(())
}

#[test]
fn semantic_collection_is_admitted_before_element_deserialization() -> TestResult {
    let p = compiled(&fixture(8, 8, 8)?, options()?)?;
    let mut input = files(&p);
    let (_, bytes) = input
        .iter_mut()
        .find(|(name, _)| name == "semantic.json")
        .ok_or("semantic")?;
    let mut value: serde_json::Value = serde_json::from_slice(bytes)?;
    value["elements"] = serde_json::json!(vec![
        serde_json::Value::Null;
        vw_semantics::MAX_ELEMENTS + 1
    ]);
    *bytes = serde_json::to_vec(&value)?;
    let hash = digest(bytes);
    change_manifest(&mut input, |v| {
        if let Some(entries) = v["files"].as_array_mut() {
            for entry in entries {
                if entry["path"] == "semantic.json" {
                    entry["sha256"] = serde_json::json!(hash);
                }
            }
        }
    })?;
    assert!(matches!(
        Package::from_files(input, Limits::default(), &NeverCancel),
        Err(Error::Limit("JSON collection"))
    ));
    Ok(())
}

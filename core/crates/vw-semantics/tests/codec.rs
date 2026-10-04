mod common;
use common::*;
use std::io::Write;
use vw_semantics::*;

fn compressed(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3)?;
    encoder.window_log(20)?;
    encoder.include_checksum(true)?;
    encoder.write_all(bytes)?;
    encoder.finish()
}
#[test]
fn trailing_frames_truncation_skippable_frames_and_corruption_are_refused() -> TestResult {
    let project = stored()?.project().clone();
    let good = payload(&project)?;
    let mut doubled = good.clone();
    doubled.extend_from_slice(&good);
    let mut suffix = good.clone();
    suffix.extend_from_slice(b"hidden");
    let mut corrupted = good.clone();
    let last = corrupted.len() - 1;
    corrupted[last] ^= 0xff;
    for bytes in [
        doubled,
        suffix,
        corrupted,
        good[..good.len() - 2].to_vec(),
        vec![0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0],
    ] {
        assert!(load(&with_payload(project.clone(), bytes)?).is_err());
    }
    Ok(())
}
#[test]
fn decompression_bomb_large_window_and_compressed_admission_are_bounded() -> TestResult {
    let project = stored()?.project().clone();
    let bomb = compressed(&vec![b' '; MAX_JSON_BYTES + 1])?;
    assert!(matches!(
        load(&with_payload(project.clone(), bomb)?),
        Err(Error::Limit("decompressed data"))
    ));
    // Ordinary Zstd magic, non-single-segment descriptor, 32MiB window. The
    // native decoder rejects that advertised window under our 1MiB cap.
    let large_window = vec![0x28, 0xb5, 0x2f, 0xfd, 0x00, 0x78, 0x01, 0x00, 0x00];
    assert!(load(&with_payload(project.clone(), large_window)?).is_err());
    assert!(matches!(
        load(&with_payload(project, vec![0; MAX_COMPRESSED_BYTES + 1])?),
        Err(Error::Limit("compressed data"))
    ));
    Ok(())
}
#[test]
fn invalid_json_schema_trust_and_envelope_mismatch_are_refused() -> TestResult {
    let project = stored()?.project().clone();
    let original = zstd::stream::decode_all(payload(&project)?.as_slice())?;
    let duplicate = std::str::from_utf8(&original)?.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    assert!(
        load(&with_payload(
            project.clone(),
            compressed(duplicate.as_bytes())?
        )?)
        .is_err()
    );
    for bytes in [
        b"{}".to_vec(),
        b"{\"a\":NaN}".to_vec(),
        [original.clone(), b" ".to_vec()].concat(),
    ] {
        assert!(load(&with_payload(project.clone(), compressed(&bytes)?)?).is_err());
    }
    for (field, value) in [
        ("schema_version", serde_json::json!(2)),
        ("text_is_untrusted", serde_json::json!(false)),
        ("platform", serde_json::json!("android_ax")),
        ("unexpected", serde_json::json!(true)),
    ] {
        let mut data: serde_json::Value = serde_json::from_slice(&original)?;
        data[field] = value;
        assert!(
            load(&with_payload(
                project.clone(),
                compressed(&serde_json::to_vec(&data)?)?
            )?)
            .is_err()
        );
    }
    let mut mismatch = project.clone();
    mismatch
        .semantic_snapshots
        .get_mut(&id(10)?)
        .ok_or("snapshot")?
        .definition
        .frame_delta_ms = 0;
    assert!(matches!(load(&mismatch), Err(Error::Stale)));
    Ok(())
}
#[test]
fn deserialization_refuses_element_count_before_unbounded_vector_growth() -> TestResult {
    let project = stored()?.project().clone();
    let original = zstd::stream::decode_all(payload(&project)?.as_slice())?;
    let mut data: serde_json::Value = serde_json::from_slice(&original)?;
    let element = data["elements"][0].clone();
    data["elements"] = serde_json::Value::Array(vec![element; MAX_ELEMENTS + 1]);
    let json = serde_json::to_vec(&data)?;
    assert!(json.len() < MAX_JSON_BYTES);
    assert!(matches!(
        load(&with_payload(project, compressed(&json)?)?),
        Err(Error::Json)
    ));
    Ok(())
}

#[path = "support/golden.rs"]
mod golden;
mod support;
use support::*;
#[test]
fn committed_png_and_layout_manifest_matches_byte_for_byte() -> TestResult {
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("goldens.json"))?;
    assert_eq!(
        expected.len(),
        9,
        "Generate and review the initial manifest centrally before accepting this task"
    );
    assert_eq!(golden::hashes()?, expected);
    Ok(())
}

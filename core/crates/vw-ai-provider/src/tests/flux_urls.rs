use super::*;

#[test]
fn raw_cost_is_only_the_unambiguous_top_level_json_number() {
    for (json, expected) in [
        (
            r#"{"result":{"cost":0.5},"note":"comma, and } quote \" stay data","cost":0.25}"#,
            Some("0.25"),
        ),
        (r#"{"cost":1e-400}"#, Some("1e-400")),
        (
            r#"{"cost":0.00010000000000000000000001}"#,
            Some("0.00010000000000000000000001"),
        ),
        (r#"{"cost":null}"#, Some("null")),
        (r#"{"result":{"cost":0.5}}"#, None),
        (r#"{"cost":0.25,"cost":0.5}"#, None),
        (r#"{"cost":0.25,"c\u006fst":0.5}"#, None),
    ] {
        assert_eq!(raw_cost_decimal(json.as_bytes()), expected, "{json}");
    }
}

#[test]
fn bfl_urls_require_https_scoped_hosts_and_exact_task_binding() -> Result<()> {
    polling_url(
        "https://api.us1.bfl.ai/v1/get_result?id=fixture-task",
        "fixture-task",
    )?;
    sample_url("https://delivery.eu1.bfl.ai/results/fixture.png?signature=fixture")?;
    for url in [
        "http://api.bfl.ai/v1/get_result?id=fixture-task",
        "https://api.bfl.ai.evil.invalid/v1/get_result?id=fixture-task",
        "https://api.bfl.ai@evil.invalid/v1/get_result?id=fixture-task",
        "https://api.bfl.ai:444/v1/get_result?id=fixture-task",
        "https://api.bfl.ai/v1/get_result?id=wrong-task",
        "https://api.bfl.ai/v1/get_result?id=fixture-task&id=fixture-task",
        "https://api.bfl.ai/v1/get_result?id=fixture-task#fragment",
        "https://api.bfl.ai/v1/flux-pro-1.0-fill?id=fixture-task",
        "https://127.0.0.1/v1/get_result?id=fixture-task",
    ] {
        assert!(polling_url(url, "fixture-task").is_err());
    }
    for url in [
        "http://delivery.eu1.bfl.ai/result.png",
        "https://delivery.eu1.bfl.ai.evil.invalid/result.png",
        "https://delivery.eu1.bfl.ai@evil.invalid/result.png",
        "https://api.bfl.ai/result.png",
        "https://delivery.bfl.ai/result.png",
        "https://delivery.eu1.bfl.ai:444/result.png",
        "https://delivery.eu1.bfl.ai/result.png#fragment",
    ] {
        assert!(sample_url(url).is_err());
    }
    Ok(())
}

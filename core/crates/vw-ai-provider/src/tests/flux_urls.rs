use super::*;
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

//! Failures that reach the apps as a coarse error keep a readable, static
//! cause in the core diagnostics, and timed stages record their duration.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;
use support::*;
use vw_core::*;

#[tokio::test]
async fn a_failed_open_and_a_timed_export_are_explained_without_runtime_text() {
    let root = tempfile::tempdir().unwrap();
    let before = core_diagnostics(0).latest_sequence;

    // Opening a folder that holds no project fails with a coarse public error.
    let missing = root.path().join("secret-owner-folder-name");
    std::fs::create_dir(&missing).unwrap();
    assert!(
        open_project(missing.to_string_lossy().into(), cancel())
            .await
            .is_err()
    );
    let after_failure = core_diagnostics(before);
    let failures: Vec<_> = after_failure
        .events
        .iter()
        .filter(|event| event.kind == CoreDiagnosticKind::Failure)
        .collect();
    assert!(
        !failures.is_empty(),
        "the failed open must leave a cause behind"
    );

    // A successful export records how long it took and how large it was.
    let project = create_image_project(create(&root.path().join("project")), cancel())
        .await
        .unwrap();
    let exported = draw(&project).await;
    let events = core_diagnostics(before).events;
    assert!(events.iter().any(|event| {
        event.kind == CoreDiagnosticKind::DurationMicros
            && event.area == "export"
            && event.name == "image"
    }));
    assert!(events.iter().any(|event| {
        event.kind == CoreDiagnosticKind::Count
            && event.area == "export"
            && event.value == exported.bytes.len() as u64
    }));

    // Nothing recorded can contain run-time text such as the folder name.
    for event in &events {
        for text in [&event.area, &event.name] {
            assert!(!text.contains("secret-owner-folder-name"));
            assert!(
                text.len() <= 64
                    && text.bytes().all(|byte| byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || b" -_.".contains(&byte)),
                "unexpected diagnostic text: {text}"
            );
        }
    }
    project.close().await.unwrap();
}

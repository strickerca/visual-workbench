use vw_capture::*;
fn target() -> WindowTarget {
    WindowTarget {
        window: 4,
        process_id: 8,
        process_created: 10,
        client: Rect {
            x: -1900,
            y: 24,
            width: 1800,
            height: 1000,
        },
        frame: Rect {
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        },
        dpi: 144,
        observed_ns: 1000,
    }
}
fn temp() -> tempfile::TempDir {
    #[cfg(target_os = "android")]
    {
        tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap()
    }
    #[cfg(not(target_os = "android"))]
    {
        tempfile::tempdir().unwrap()
    }
}
#[test]
fn physical_client_offsets_preserve_negative_monitor_origins() {
    assert_eq!(
        target().client.offset_inside(target().frame).unwrap(),
        (20, 24)
    );
}
#[test]
fn handle_reuse_resize_dpi_and_foreground_identity_refuse() {
    let old = target();
    let mut current = old.clone();
    current.process_created += 1;
    assert_eq!(old.unchanged(&current, 2), Err(Error::Stale));
    current = old.clone();
    current.dpi = 96;
    assert_eq!(old.unchanged(&current, 2), Err(Error::Stale));
    current = old.clone();
    current.client.width -= 1;
    assert_eq!(old.unchanged(&current, 2), Err(Error::Stale));
}
#[test]
fn own_window_refuses_without_calling_capture() {
    assert_eq!(target().validate(8), Err(Error::OwnWindow));
}
#[test]
fn frame_timing_retains_direction_and_checks_overflow() {
    assert_eq!(frame_delta_ms(10_000_000, 15_000_000).unwrap(), 5);
    assert_eq!(frame_delta_ms(15_000_000, 10_000_000).unwrap(), -5);
    assert_eq!(frame_delta_ms(0, u64::MAX), Err(Error::Limit));
}
#[test]
fn allocation_admission_precedes_any_pixel_copy() {
    assert_eq!(
        Limits::default().image(u32::MAX, u32::MAX),
        Err(Error::Limit)
    );
    assert_eq!(Limits::default().image(6000, 6000), Err(Error::Limit));
    assert_eq!(
        Limits::default().image(1920, 1080).unwrap(),
        1920 * 1080 * 4
    );
}
#[test]
fn no_implicit_extended_tree_deadline() {
    assert_eq!(
        Limits {
            tree_ms: 301,
            ..Limits::default()
        }
        .validate()
        .err(),
        Some(Error::Limit)
    );
}
#[test]
fn text_and_nodes_have_independent_caps() {
    let value = Element {
        local_id: "x".into(),
        parent_local_id: None,
        name: "button".into(),
        role: "button".into(),
        automation_id: None,
        resource_id: None,
        html_id: None,
        bounds: [0.0, 0.0, 1.0, 1.0],
        text: "a".repeat(201),
        enabled: true,
        focused: false,
    };
    assert_eq!(
        admit_element(&value, &mut 0, Limits::default()),
        Err(Error::Limit)
    );
}
#[test]
fn protected_helper_rejects_unbound_hash_before_spawn() {
    let directory = temp();
    let file = directory.path().join("helper.exe");
    std::fs::write(&file, b"not an executable").unwrap();
    assert!(process::LockedHelper::open(&file, &"0".repeat(64)).is_err());
}

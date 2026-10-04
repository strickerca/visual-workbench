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
#[test]
fn refresh_allows_new_geometry_only_for_the_same_selected_process_identity() {
    let old = target();
    let mut fresh = old.clone();
    fresh.client.width = 1700;
    fresh.dpi = 192;
    fresh.observed_ns += 1;
    let updated = refreshed_target(&old, fresh.clone(), 2).unwrap();
    assert_eq!(updated, fresh);
    assert_eq!(old.unchanged(&updated, 2), Err(Error::Stale));
}
#[test]
fn recycled_window_or_process_and_clock_rollback_require_new_selection() {
    let old = target();
    for field in 0..4 {
        let mut fresh = old.clone();
        match field {
            0 => fresh.window += 1,
            1 => fresh.process_id += 1,
            2 => fresh.process_created += 1,
            _ => fresh.observed_ns -= 1,
        };
        assert_eq!(refreshed_target(&old, fresh, 2), Err(Error::Stale));
    }
}
#[test]
fn own_window_and_invalid_fresh_geometry_are_never_refreshed() {
    let old = target();
    assert_eq!(
        refreshed_target(&old, old.clone(), 8),
        Err(Error::OwnWindow)
    );
    let mut fresh = old.clone();
    fresh.client.width = u32::MAX;
    assert!(refreshed_target(&old, fresh, 2).is_err());
}

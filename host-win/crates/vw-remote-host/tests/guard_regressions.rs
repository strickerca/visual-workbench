#[path = "../src/platform/guard.rs"]
mod production;
use production::{Guard, Rect, Stop, Target};

fn target() -> Target {
    Target {
        hwnd: 20,
        pid: 30,
        thread: 40,
        process_created: 50,
        window: Rect {
            left: -100,
            top: -50,
            right: 900,
            bottom: 750,
        },
        client: Rect {
            left: -90,
            top: -20,
            right: 890,
            bottom: 740,
        },
        dpi: 144,
        integrity: 0x2000,
    }
}
fn armed() -> Result<Guard, Stop> {
    Guard::new(target(), 7, 0x2000)
}

#[test]
fn exact_target_accepts_negative_physical_coordinates_and_half_open_edges() -> Result<(), Stop> {
    let mut g = armed()?;
    let t = target();
    assert_eq!(g.check(t, t.hwnd, t.hwnd, 7, (-90, -20)), Ok(()));
    assert_eq!(g.check(t, t.hwnd, t.hwnd, 7, (889, 739)), Ok(()));
    assert!(!g.suspended());
    assert_eq!(
        g.check(t, t.hwnd, t.hwnd, 7, (890, 739)),
        Err(Stop::OutsideClient)
    );
    assert!(g.suspended());
    Ok(())
}
#[test]
fn every_retained_identity_and_geometry_change_latches_before_injection() -> Result<(), Stop> {
    let t = target();
    let mut changes = Vec::new();
    let mut v = t;
    v.hwnd += 1;
    changes.push(v);
    let mut v = t;
    v.pid += 1;
    changes.push(v);
    let mut v = t;
    v.thread += 1;
    changes.push(v);
    let mut v = t;
    v.process_created += 1;
    changes.push(v);
    let mut v = t;
    v.window.left += 1;
    changes.push(v);
    let mut v = t;
    v.window.right -= 1;
    changes.push(v);
    let mut v = t;
    v.client.top += 1;
    changes.push(v);
    let mut v = t;
    v.client.bottom -= 1;
    changes.push(v);
    let mut v = t;
    v.dpi = 192;
    changes.push(v);
    let mut v = t;
    v.integrity = 0x1000;
    changes.push(v);
    for current in changes {
        let mut g = armed()?;
        assert_eq!(
            g.check(current, t.hwnd, t.hwnd, 7, (0, 0)),
            Err(Stop::TargetChanged)
        );
        assert_eq!(g.check(t, t.hwnd, t.hwnd, 7, (0, 0)), Err(Stop::Suspended));
    }
    Ok(())
}
#[test]
fn foreground_and_actual_destination_occlusion_latch_independently() -> Result<(), Stop> {
    let t = target();
    let mut g = armed()?;
    assert_eq!(
        g.check(t, 99, t.hwnd, 7, (0, 0)),
        Err(Stop::ForegroundChanged)
    );
    assert_eq!(g.check(t, t.hwnd, t.hwnd, 7, (0, 0)), Err(Stop::Suspended));
    let mut g = armed()?;
    assert_eq!(g.check(t, t.hwnd, 99, 7, (700, 600)), Err(Stop::Occluded));
    assert!(g.suspended());
    Ok(())
}
#[test]
fn reconnect_cannot_reuse_latched_session_and_requires_new_guard() -> Result<(), Stop> {
    let t = target();
    let mut old = armed()?;
    assert_eq!(
        old.check(t, t.hwnd, t.hwnd, 8, (0, 0)),
        Err(Stop::SessionChanged)
    );
    assert_eq!(
        old.check(t, t.hwnd, t.hwnd, 7, (0, 0)),
        Err(Stop::Suspended)
    );
    let mut fresh = Guard::new(t, 8, 0x2000)?;
    assert_eq!(fresh.check(t, t.hwnd, t.hwnd, 8, (0, 0)), Ok(()));
    Ok(())
}
#[test]
fn failed_os_snapshot_is_a_latched_revoke_not_an_implicit_new_grant() -> Result<(), Stop> {
    let mut g = armed()?;
    let t = target();
    g.suspend();
    assert_eq!(g.check(t, t.hwnd, t.hwnd, 7, (0, 0)), Err(Stop::Suspended));
    Ok(())
}
#[test]
fn integrity_rise_and_invalid_initial_target_are_refused() -> Result<(), Stop> {
    let t = target();
    let mut v = t;
    v.integrity = 0x3000;
    assert!(matches!(
        Guard::new(v, 7, 0x2000),
        Err(Stop::HigherIntegrity)
    ));
    let mut g = armed()?;
    assert_eq!(
        g.check(v, t.hwnd, t.hwnd, 7, (0, 0)),
        Err(Stop::HigherIntegrity)
    );
    for invalid in [
        Target { hwnd: 0, ..t },
        Target {
            process_created: 0,
            ..t
        },
        Target {
            client: Rect {
                left: -101,
                ..t.client
            },
            ..t
        },
        Target {
            window: Rect {
                left: i32::MIN,
                right: i32::MAX,
                ..t.window
            },
            ..t
        },
    ] {
        assert!(matches!(
            Guard::new(invalid, 7, 0x2000),
            Err(Stop::InvalidTarget)
        ));
    }
    Ok(())
}

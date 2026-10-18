#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vw_capture::*;
use vw_capture::picker::*;
use vw_geom::{GeometryRevision, PixelPoint};
fn monitor() -> MonitorTarget { MonitorTarget { monitor: 1, device: "synthetic".into(),
    physical: Rect { x: -1440, y: -200, width: 1440, height: 2560 }, dpi: 144,
    rotation: 1, primary: false, observed_ns: 10 } }

#[test]
fn physical_mapping_is_exact_and_dpi_is_only_metadata() {
    for dpi in [96, 120, 144, 168, 192, 240, 288] { for rotation in 0..4 {
        let mut target = monitor(); target.dpi = dpi; target.rotation = rotation;
        let source = PickerSource::Monitor { target }; let g = source.geometry(7).unwrap();
        for point in [PixelPoint { x: 0, y: 0 }, PixelPoint { x: 1439, y: 2559 }, PixelPoint { x: 719, y: 1269 }] {
            let host = g.frame_to_host(point, GeometryRevision(7)).unwrap();
            assert_eq!(host, PixelPoint { x: point.x - 1440, y: point.y - 200 });
            assert_eq!(g.host_to_frame(host, GeometryRevision(7)).unwrap(), point);
            assert!(g.frame_to_host(point, GeometryRevision(8)).is_err());
        }
    } }
}
#[test]
fn region_uses_pixel_edges_and_refuses_cross_monitor_silent_clipping() {
    let source = PickerSource::Monitor { target: monitor() };
    assert_eq!(source.region_from_frame_edges([20.9, 30.1, 10.1, 12.9], 1).unwrap(),
        Rect { x: -1430, y: -188, width: 11, height: 19 });
    for edges in [[-0.1, 0.0, 10.0, 10.0], [0.0, 0.0, 1440.1, 20.0],
        [0.0, 0.0, 0.0, 0.0], [f64::NAN, 0.0, 2.0, 2.0]] {
        assert!(source.region_from_frame_edges(edges, 1).is_err());
    }
}
#[test]
fn source_identity_and_geometry_changes_never_retarget_an_old_selection() {
    let original = monitor(); let mut fresh = original.clone(); fresh.observed_ns += 1;
    original.unchanged(&fresh).unwrap();
    for kind in 0..7 { let mut changed = fresh.clone(); match kind {
        0 => changed.monitor += 1, 1 => changed.device.push('x'), 2 => changed.physical.x += 1,
        3 => changed.physical.width += 1, 4 => changed.dpi = 192, 5 => changed.rotation = 0,
        _ => changed.observed_ns = 9,
    }; assert_eq!(original.unchanged(&changed), Err(Error::Stale)); }
}
#[test]
fn region_capture_charges_full_monitor_readback_before_crop() {
    let source = PickerSource::Region { target: monitor(), region: Rect { x: -1440, y: -200, width: 1, height: 1 } };
    assert_eq!(source.validate(99, Limits { memory_bytes: 32 * 1024 * 1024, ..Limits::default() }), Err(Error::Limit));
}
#[test]
fn distinct_window_monitor_and_region_types_cannot_silently_broaden() {
    let source = PickerSource::Region { target: monitor(), region: Rect { x: -1440, y: -200, width: 30, height: 40 } };
    assert_eq!(source.kind(), "region"); assert_eq!(source.rect().width, 30);
    let bytes = serde_json::to_vec(&source).unwrap();
    assert_eq!(serde_json::from_slice::<PickerSource>(&bytes).unwrap(), source);
    assert!(serde_json::from_str::<PickerSource>("{\"kind\":\"window\",\"target\":null,\"region\":{}}").is_err());
}
#[test]
fn consent_zero_identity_and_cancel_are_rejected_before_platform_work() {
    let mut request = PickerCaptureRequest { explicit_owner_action: false, owner_process_id: 99,
        source: PickerSource::Monitor { target: monitor() }, capture_session_id: "019f0000-0000-7000-8000-000000000001".into(),
        frame_id: 1, geometry_revision: 1, output_directory: "fixture".into(), limits: Limits::default() };
    let cancel = Cancellation::default(); assert_eq!(request.validate(&cancel), Err(Error::Consent));
    request.explicit_owner_action = true; request.validate(&cancel).unwrap();
    request.frame_id = 0; assert_eq!(request.validate(&cancel), Err(Error::Invalid));
    cancel.cancel(); assert_eq!(request.validate(&cancel), Err(Error::Cancelled));
}

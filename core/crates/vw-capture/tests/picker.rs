#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vw_capture::picker::*;
use vw_capture::*;
use vw_geom::{GeometryRevision, PixelPoint};
fn monitor() -> MonitorTarget {
    MonitorTarget {
        monitor: 1,
        device: "synthetic".into(),
        physical: Rect {
            x: -1440,
            y: -200,
            width: 1440,
            height: 2560,
        },
        dpi: 144,
        rotation: 1,
        primary: false,
        observed_ns: 10,
    }
}

#[test]
fn physical_mapping_is_exact_and_dpi_is_only_metadata() {
    for dpi in [96, 120, 144, 168, 192, 240, 288] {
        for rotation in 0..4 {
            let mut target = monitor();
            target.dpi = dpi;
            target.rotation = rotation;
            let source = PickerSource::Monitor { target };
            let g = source.geometry(7).unwrap();
            for point in [
                PixelPoint { x: 0, y: 0 },
                PixelPoint { x: 1439, y: 2559 },
                PixelPoint { x: 719, y: 1269 },
            ] {
                let host = g.frame_to_host(point, GeometryRevision(7)).unwrap();
                assert_eq!(
                    host,
                    PixelPoint {
                        x: point.x - 1440,
                        y: point.y - 200
                    }
                );
                assert_eq!(g.host_to_frame(host, GeometryRevision(7)).unwrap(), point);
                assert!(g.frame_to_host(point, GeometryRevision(8)).is_err());
            }
        }
    }
}
#[test]
fn region_uses_pixel_edges_and_refuses_cross_monitor_silent_clipping() {
    let source = PickerSource::Monitor { target: monitor() };
    assert_eq!(
        source
            .region_from_frame_edges([20.9, 30.1, 10.1, 12.9], 1)
            .unwrap(),
        Rect {
            x: -1430,
            y: -188,
            width: 11,
            height: 19
        }
    );
    for edges in [
        [-0.1, 0.0, 10.0, 10.0],
        [0.0, 0.0, 1440.1, 20.0],
        [0.0, 0.0, 0.0, 0.0],
        [f64::NAN, 0.0, 2.0, 2.0],
    ] {
        assert!(source.region_from_frame_edges(edges, 1).is_err());
    }
}
#[test]
fn source_identity_and_geometry_changes_never_retarget_an_old_selection() {
    let original = monitor();
    let mut fresh = original.clone();
    fresh.observed_ns += 1;
    original.unchanged(&fresh).unwrap();
    for kind in 0..8 {
        let mut changed = fresh.clone();
        match kind {
            0 => changed.monitor += 1,
            1 => changed.device.push('x'),
            2 => changed.physical.x += 1,
            3 => changed.physical.width += 1,
            4 => changed.dpi = 192,
            5 => changed.rotation = 0,
            6 => changed.primary = true,
            _ => changed.observed_ns = 9,
        };
        assert_eq!(original.unchanged(&changed), Err(Error::Stale));
    }
}
#[test]
fn region_capture_charges_full_monitor_readback_before_crop() {
    let source = PickerSource::Region {
        target: monitor(),
        region: Rect {
            x: -1440,
            y: -200,
            width: 1,
            height: 1,
        },
    };
    assert_eq!(
        source.validate(
            99,
            Limits {
                memory_bytes: 32 * 1024 * 1024,
                ..Limits::default()
            }
        ),
        Err(Error::Limit)
    );
}
#[test]
fn distinct_window_monitor_and_region_types_cannot_silently_broaden() {
    let source = PickerSource::Region {
        target: monitor(),
        region: Rect {
            x: -1440,
            y: -200,
            width: 30,
            height: 40,
        },
    };
    assert_eq!(source.kind(), "region");
    assert_eq!(source.rect().width, 30);
    let bytes = serde_json::to_vec(&source).unwrap();
    assert_eq!(
        serde_json::from_slice::<PickerSource>(&bytes).unwrap(),
        source
    );
    assert!(
        serde_json::from_str::<PickerSource>("{\"kind\":\"window\",\"target\":null,\"region\":{}}")
            .is_err()
    );
}
#[test]
fn consent_zero_identity_and_cancel_are_rejected_before_platform_work() {
    let mut request = PickerCaptureRequest {
        explicit_owner_action: false,
        owner_process_id: 99,
        source: PickerSource::Monitor { target: monitor() },
        capture_session_id: "019f0000-0000-7000-8000-000000000001".into(),
        frame_id: 1,
        geometry_revision: 1,
        output_directory: "fixture".into(),
        limits: Limits::default(),
    };
    let cancel = Cancellation::default();
    assert_eq!(request.validate(&cancel), Err(Error::Consent));
    request.explicit_owner_action = true;
    request.validate(&cancel).unwrap();
    request.frame_id = 0;
    assert_eq!(request.validate(&cancel), Err(Error::Invalid));
    cancel.cancel();
    assert_eq!(request.validate(&cancel), Err(Error::Cancelled));
}

fn window() -> WindowTarget {
    WindowTarget {
        window: 7,
        process_id: 8,
        process_created: 9,
        client: Rect {
            x: -90,
            y: -30,
            width: 80,
            height: 60,
        },
        frame: Rect {
            x: -100,
            y: -50,
            width: 100,
            height: 90,
        },
        dpi: 192,
        observed_ns: 10,
    }
}
fn capture_request(source: PickerSource) -> PickerCaptureRequest {
    PickerCaptureRequest {
        explicit_owner_action: true,
        owner_process_id: 99,
        source,
        capture_session_id: "019f0000-0000-7000-8000-000000000001".into(),
        frame_id: 7,
        geometry_revision: 3,
        output_directory: "fixture".into(),
        limits: Limits::default(),
    }
}
fn receipt(request: &PickerCaptureRequest) -> PickerFrameReceipt {
    let rect = request.source.rect();
    let (window_handle, monitor_id) = match &request.source {
        PickerSource::Window { target } => (target.window, String::new()),
        PickerSource::Monitor { target } | PickerSource::Region { target, .. } => {
            (0, target.device.clone())
        }
    };
    let identity = FrameIdentity {
        capture_session_id: request.capture_session_id.clone(),
        frame_id: request.frame_id,
        geometry_revision: request.geometry_revision,
        source_kind: request.source.kind().into(),
        client_rect: rect,
        dpi_scale: f64::from(request.source.dpi()) / 96.0,
        monotonic_timestamp_ns: request.source.observed_ns() + 1,
        captured_at_ms: 20,
        platform: "windows".into(),
        window_handle,
        monitor_id,
    };
    let source_asset_id = "0".repeat(64);
    let window_receipt = match &request.source {
        PickerSource::Window { target } => Some(FrameReceipt {
            identity: identity.clone(),
            target: target.clone(),
            source_asset_id: source_asset_id.clone(),
            png_bytes: 100,
            width: rect.width,
            height: rect.height,
            bit_depth: 8,
            border_visible: true,
            lossless: true,
            filename: "capture.png".into(),
        }),
        _ => None,
    };
    PickerFrameReceipt {
        identity,
        source: request.source.clone(),
        source_asset_id,
        png_bytes: 100,
        width: rect.width,
        height: rect.height,
        bit_depth: 8,
        border_visible: true,
        filename: "capture.png".into(),
        window_receipt,
    }
}

#[test]
fn fractional_clicks_and_lines_do_not_expand_into_selections() {
    let source = PickerSource::Monitor { target: monitor() };
    for edges in [
        [0.1, 0.2, 0.1, 0.2],
        [0.1, 0.2, 0.1, 1.2],
        [0.1, 0.2, 1.1, 0.2],
        [1439.9, 2559.9, 1439.9, 2559.9],
    ] {
        assert_eq!(
            source.region_from_frame_edges(edges, 1),
            Err(Error::Invalid)
        );
    }
    assert_eq!(
        source
            .region_from_frame_edges([0.1, 0.2, 0.2, 0.3], 1)
            .unwrap(),
        Rect {
            x: -1440,
            y: -200,
            width: 1,
            height: 1
        }
    );
}

#[test]
fn complete_half_open_extent_and_last_pixel_map_without_clipping() {
    let target = monitor();
    let source = PickerSource::Monitor {
        target: target.clone(),
    };
    assert_eq!(
        source
            .region_from_frame_edges([1440.0, 2560.0, 0.0, 0.0], 1)
            .unwrap(),
        target.physical
    );
    assert_eq!(
        source
            .region_from_frame_edges([1439.1, 2559.1, 1440.0, 2560.0], 1)
            .unwrap(),
        Rect {
            x: -1,
            y: 2359,
            width: 1,
            height: 1
        }
    );
    for edges in [
        [0.0, 0.0, f64::MAX, 1.0],
        [f64::NEG_INFINITY, 0.0, 1.0, 1.0],
        [0.0, 0.0, 1.0, f64::INFINITY],
        [0.0, 0.0, 1.0, 2560.1],
    ] {
        assert_eq!(
            source.region_from_frame_edges(edges, 1),
            Err(Error::Invalid)
        );
    }
    assert_eq!(
        source.region_from_frame_edges([0.0, 0.0, 1.0, 1.0], 0),
        Err(Error::Invalid)
    );
}

#[test]
fn geometry_rejects_unrepresentable_frame_indices_before_integer_casts() {
    let mut target = monitor();
    target.physical = Rect {
        x: i32::MIN,
        y: 0,
        width: u32::MAX,
        height: 1,
    };
    let source = PickerSource::Monitor { target };
    assert_eq!(source.geometry(1), Err(Error::Invalid));
    assert_eq!(
        source.region_from_frame_edges([0.0, 0.0, f64::from(u32::MAX), 1.0], 1),
        Err(Error::Invalid)
    );
}

#[test]
fn region_geometry_stays_inside_its_original_monitor() {
    let source = PickerSource::Region {
        target: monitor(),
        region: Rect {
            x: -1430,
            y: -180,
            width: 40,
            height: 30,
        },
    };
    source.validate(99, Limits::default()).unwrap();
    assert_eq!(
        source
            .region_from_frame_edges([0.0, 0.0, 40.0, 30.0], 1)
            .unwrap(),
        source.rect()
    );
    assert!(
        source
            .region_from_frame_edges([0.0, 0.0, 41.0, 30.0], 1)
            .is_err()
    );
    let outside = PickerSource::Region {
        target: monitor(),
        region: Rect {
            x: -1441,
            y: -200,
            width: 2,
            height: 2,
        },
    };
    assert_eq!(outside.geometry(1), Err(Error::Stale));
    assert_eq!(outside.validate(99, Limits::default()), Err(Error::Stale));
}

#[test]
fn malformed_monitor_metadata_is_never_a_capture_or_mapping_source() {
    for kind in 0..9 {
        let mut target = monitor();
        match kind {
            0 => target.monitor = 0,
            1 => target.device.clear(),
            2 => target.device.push('\0'),
            3 => target.device = "x".repeat(129),
            4 => target.dpi = 47,
            5 => target.dpi = 961,
            6 => target.rotation = 4,
            7 => target.observed_ns = 0,
            _ => target.observed_ns = i64::MAX as u64 + 1,
        }
        assert_eq!(target.validate(), Err(Error::Invalid));
        let source = PickerSource::Monitor { target };
        assert_eq!(source.validate(99, Limits::default()), Err(Error::Invalid));
        assert_eq!(source.geometry(1), Err(Error::Invalid));
    }
}

#[cfg(target_pointer_width = "32")]
#[test]
fn foreign_handle_values_must_fit_the_native_pointer_width() {
    let mut target = monitor();
    target.monitor = u64::from(u32::MAX) + 1;
    assert_eq!(target.validate(), Err(Error::Invalid));
    let mut target = window();
    target.window = u64::from(u32::MAX) + 1;
    assert_eq!(
        PickerSource::Window { target }.validate(99, Limits::default()),
        Err(Error::Invalid)
    );
}

#[test]
fn own_windows_and_zero_owner_cannot_enter_any_picker_source() {
    let source = PickerSource::Window { target: window() };
    assert_eq!(source.validate(8, Limits::default()), Err(Error::OwnWindow));
    for source in [
        source,
        PickerSource::Monitor { target: monitor() },
        PickerSource::Region {
            target: monitor(),
            region: monitor().physical,
        },
    ] {
        assert_eq!(source.validate(0, Limits::default()), Err(Error::Invalid));
    }
    let catalog = PickerCatalog {
        windows: vec![],
        monitors: vec![],
        truncated: false,
    };
    assert_eq!(catalog.validate(0), Err(Error::Invalid));
}

#[test]
fn catalog_rejects_duplicate_sources_private_extra_fields_and_unbounded_labels() {
    let mut catalog = PickerCatalog {
        windows: vec![PickerWindow {
            title: "fixture".into(),
            target: window(),
        }],
        monitors: vec![monitor()],
        truncated: false,
    };
    catalog.validate(99).unwrap();
    catalog.windows.push(catalog.windows[0].clone());
    assert_eq!(catalog.validate(99), Err(Error::Invalid));
    catalog.windows.pop();
    catalog.monitors.push(monitor());
    assert_eq!(catalog.validate(99), Err(Error::Invalid));
    catalog.monitors.pop();
    for title in ["x".repeat(4097), "fixture\0label".into()] {
        catalog.windows[0].title = title;
        assert_eq!(catalog.validate(99), Err(Error::Invalid));
    }
    let mut value = serde_json::to_value(&catalog).unwrap();
    value["private_path"] = "fixture".into();
    assert!(serde_json::from_value::<PickerCatalog>(value).is_err());
}

#[test]
fn catalog_counts_are_bounded_even_when_truncated_is_claimed() {
    let catalog = PickerCatalog {
        windows: vec![],
        monitors: vec![monitor(); MAX_PICKER_MONITORS + 1],
        truncated: true,
    };
    assert_eq!(catalog.validate(99), Err(Error::Limit));
    let catalog = PickerCatalog {
        windows: vec![
            PickerWindow {
                title: String::new(),
                target: window()
            };
            MAX_PICKER_WINDOWS + 1
        ],
        monitors: vec![],
        truncated: true,
    };
    assert_eq!(catalog.validate(99), Err(Error::Limit));
}

#[test]
fn malformed_capture_identity_limits_and_output_are_rejected() {
    for kind in 0..9 {
        let mut request = capture_request(PickerSource::Monitor { target: monitor() });
        match kind {
            0 => request.owner_process_id = 0,
            1 => request.capture_session_id = "invalid".into(),
            2 => request.frame_id = 0,
            3 => request.frame_id = i64::MAX as u64 + 1,
            4 => request.geometry_revision = 0,
            5 => request.output_directory.clear(),
            6 => request.output_directory = "x".repeat(4097),
            7 => request.output_directory.push('\0'),
            _ => request.limits.capture_ms = 0,
        }
        assert!(request.validate(&Cancellation::default()).is_err());
    }
}

#[test]
fn receipt_binds_exact_source_frame_geometry_and_publication_metadata() {
    let request = capture_request(PickerSource::Monitor { target: monitor() });
    let original = receipt(&request);
    original.validate(&request).unwrap();
    for kind in 0..18 {
        let mut changed = original.clone();
        match kind {
            0 => {
                changed.identity.capture_session_id = "019f0000-0000-7000-8000-000000000002".into()
            }
            1 => changed.identity.frame_id += 1,
            2 => changed.identity.geometry_revision += 1,
            3 => changed.identity.source_kind = "window".into(),
            4 => changed.identity.client_rect.x += 1,
            5 => changed.identity.dpi_scale = 1.0,
            6 => changed.identity.monotonic_timestamp_ns = 9,
            7 => changed.identity.platform = "android".into(),
            8 => changed.identity.window_handle = 7,
            9 => changed.identity.monitor_id.push('x'),
            10 => changed.source_asset_id = "bad".into(),
            11 => changed.png_bytes = 0,
            12 => changed.png_bytes = request.limits.png_bytes + 1,
            13 => changed.width += 1,
            14 => changed.height += 1,
            15 => changed.bit_depth = 16,
            16 => changed.border_visible = false,
            _ => changed.filename = "other.png".into(),
        }
        assert_eq!(
            changed.validate(&request),
            Err(Error::Invalid),
            "receipt mutation {kind}"
        );
    }
    let mut changed = original;
    if let PickerSource::Monitor { target } = &mut changed.source {
        target.primary = true;
    }
    assert_eq!(changed.validate(&request), Err(Error::Invalid));
}

#[test]
fn receipt_does_not_bless_its_matching_but_invalid_capture_request() {
    let mut request = capture_request(PickerSource::Monitor { target: monitor() });
    let frame = receipt(&request);
    request.explicit_owner_action = false;
    assert_eq!(frame.validate(&request), Err(Error::Consent));
    request.explicit_owner_action = true;
    request.owner_process_id = 0;
    assert_eq!(frame.validate(&request), Err(Error::Invalid));
    let request = capture_request(PickerSource::Region {
        target: monitor(),
        region: Rect {
            x: -1441,
            y: -200,
            width: 1,
            height: 1,
        },
    });
    assert_eq!(receipt(&request).validate(&request), Err(Error::Stale));
}

#[test]
fn only_exact_lossless_window_receipts_can_authorize_a_semantic_subtree() {
    let window_request = capture_request(PickerSource::Window { target: window() });
    let original = receipt(&window_request);
    original.validate(&window_request).unwrap();
    for kind in 0..7 {
        let mut changed = original.clone();
        let frame = changed.window_receipt.as_mut().unwrap();
        match kind {
            0 => frame.target.process_created += 1,
            1 => frame.identity.frame_id += 1,
            2 => frame.source_asset_id = "1".repeat(64),
            3 => frame.lossless = false,
            4 => frame.width += 1,
            5 => frame.border_visible = false,
            _ => frame.filename = "other.png".into(),
        }
        assert_eq!(changed.validate(&window_request), Err(Error::Invalid));
    }
    let mut changed = original.clone();
    changed.window_receipt = None;
    assert_eq!(changed.validate(&window_request), Err(Error::Invalid));
    let mut own_request = window_request.clone();
    own_request.owner_process_id = window().process_id;
    assert_eq!(original.validate(&own_request), Err(Error::OwnWindow));
    for source in [
        PickerSource::Monitor { target: monitor() },
        PickerSource::Region {
            target: monitor(),
            region: Rect {
                x: -1440,
                y: -200,
                width: 10,
                height: 20,
            },
        },
    ] {
        let request = capture_request(source);
        let mut frame = receipt(&request);
        frame.validate(&request).unwrap();
        frame.window_receipt = original.window_receipt.clone();
        assert_eq!(frame.validate(&request), Err(Error::Invalid));
    }
}

#[test]
fn picker_operations_keep_distinct_closed_wire_discriminators() {
    let request = capture_request(PickerSource::Region {
        target: monitor(),
        region: monitor().physical,
    });
    let bytes = serde_json::to_vec(&Request::PickerCapture(request.clone())).unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"operation\":\"picker_capture\"")
    );
    assert!(matches!(
        serde_json::from_slice::<Request>(&bytes).unwrap(),
        Request::PickerCapture(_)
    ));
    let frame = receipt(&request);
    let bytes = serde_json::to_vec(&Response::PickerFrame(frame)).unwrap();
    assert!(
        std::str::from_utf8(&bytes)
            .unwrap()
            .contains("\"result\":\"picker_frame\"")
    );
    assert!(matches!(
        serde_json::from_slice::<Response>(&bytes).unwrap(),
        Response::PickerFrame(_)
    ));
    assert!(serde_json::from_str::<PickerCaptureRequest>(
        "{\"explicit_owner_action\":true,\"owner_process_id\":99,\"source\":null,\"private_path\":\"fixture\"}"
    ).is_err());
}

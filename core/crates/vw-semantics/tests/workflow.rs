mod common;
use common::*;
use vw_proto::v1;
use vw_semantics::*;

#[test]
fn canonical_snapshot_roundtrip_retry_and_undo_use_existing_ops() -> TestResult {
    let mut host = fixture()?;
    let view = view(&host)?;
    let snapshot = prepare(&view, 10, elements())?;
    let plan = view.plan(&snapshot, meta(1)?, &NeverCancel)?;
    let first = plan.submit(&mut host, &device(1), 2100)?;
    let hash = host.project().state_hash()?;
    let retry = plan.submit(&mut host, &device(1), 2200)?;
    assert_eq!(first.ack, retry.ack);
    assert_eq!(hash, host.project().state_hash()?);
    let restored = common::view(&host)?.load(&id(10)?, &NeverCancel)?;
    assert_eq!(restored.elements(), snapshot.elements());
    assert_eq!(restored.context().frame_id(), u64::MAX);
    assert_eq!(restored.frame_delta_ms(), -7);
    assert_eq!(
        restored.encode(&NeverCancel)?,
        snapshot.encode(&NeverCancel)?
    );
    let txn = v1::Transaction {
        txn_id: Some(id(20001)?.to_proto()),
        project_id: Some(id(1)?.to_proto()),
        device_id: device(1).to_string(),
        base_revision: Some(host.revision()?),
        created_at_wall_ms: 2300,
        gesture_id: None,
        ops: vec![v1::Op {
            op_id: Some(v1::OpId {
                device_id: device(1).to_string(),
                lamport: 2,
            }),
            kind: Some(v1::op::Kind::UndoTransaction(v1::UndoTransaction {
                target_txn_id: plan.transaction().txn_id.clone(),
            })),
        }],
    };
    host.submit(txn, &device(1), 2300)?;
    assert!(host.project().semantic_snapshots.is_empty());
    Ok(())
}

#[test]
fn physical_host_mapping_clips_bounds_without_applying_dpi_again() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    let snapshot = view.prepare(
        view.context(),
        id(10)?,
        Platform::ChromiumUia,
        4,
        301,
        BoundsSpace::HostPhysical,
        vec![element("button", None, [-1940.0, -90.0, 60.0, 40.0])],
        &NeverCancel,
    )?;
    assert_eq!(
        snapshot.elements()[0].bounds_document,
        [0.0, 10.0, 40.0, 40.0]
    );
    assert!(snapshot.elements()[0].bounds_clipped);
    assert_eq!(snapshot.uia_collection_within_target(), Some(false));
    assert_eq!(snapshot.collection_elapsed_ms(), 301);
    Ok(())
}

#[test]
fn plans_refuse_stale_revision_and_foreign_authentication_atomically() -> TestResult {
    let mut host = fixture()?;
    let view = view(&host)?;
    let snapshot = prepare(&view, 10, elements())?;
    let plan = view.plan(&snapshot, meta(1)?, &NeverCancel)?;
    let competing = prepare(&view, 11, elements())?;
    let other = view.plan(&competing, meta(2)?, &NeverCancel)?;
    let before = host.checkpoint_bytes()?;
    assert!(matches!(
        plan.submit(&mut host, &device(2), 2000),
        Err(Error::Unauthorized)
    ));
    assert_eq!(before, host.checkpoint_bytes()?);
    other.submit(&mut host, &device(1), 2100)?;
    let current = host.checkpoint_bytes()?;
    assert!(matches!(
        plan.submit(&mut host, &device(1), 2200),
        Err(Error::Stale)
    ));
    assert_eq!(current, host.checkpoint_bytes()?);
    Ok(())
}

#[test]
fn late_collection_and_stored_payload_cannot_cross_capture_geometry_or_frame() -> TestResult {
    let host = fixture()?;
    let old = view(&host)?.context().clone();
    for change in [0, 1, 2, 3] {
        let mut project = host.project().clone();
        let capture = project
            .documents
            .get_mut(&id(2)?)
            .ok_or("document")?
            .definition
            .capture
            .as_mut()
            .ok_or("capture")?;
        match change {
            0 => capture.frame_id = 1,
            1 => capture.capture_session_id = Some(id(4)?.to_proto()),
            2 => {
                capture
                    .geometry
                    .as_mut()
                    .ok_or("geometry")?
                    .geometry_revision += 1
            }
            _ => {
                capture
                    .geometry
                    .as_mut()
                    .ok_or("geometry")?
                    .client_rect_host
                    .as_mut()
                    .ok_or("rect")?
                    .x += 1
            }
        }
        let revision = v1::Revision {
            host_seq: 0,
            state_hash: project.state_hash()?.bytes().to_vec(),
        };
        let current = CaptureView::new(&project, &revision, &id(2)?)?;
        assert!(matches!(
            current.prepare(
                &old,
                id(10)?,
                Platform::Uia,
                0,
                1,
                BoundsSpace::CapturePixels,
                elements(),
                &NeverCancel
            ),
            Err(Error::Stale)
        ));
    }
    let mut project = stored()?.project().clone();
    project
        .documents
        .get_mut(&id(2)?)
        .ok_or("document")?
        .definition
        .capture
        .as_mut()
        .ok_or("capture")?
        .geometry
        .as_mut()
        .ok_or("geometry")?
        .dpi_scale = 2.0;
    assert!(matches!(load(&project), Err(Error::Stale)));
    Ok(())
}

#[test]
fn android_requires_matching_platform_and_screenshot_coordinates() -> TestResult {
    let mut project = fixture()?.project().clone();
    project
        .documents
        .get_mut(&id(2)?)
        .ok_or("document")?
        .definition
        .capture
        .as_mut()
        .ok_or("capture")?
        .platform = "android".into();
    let rev = v1::Revision {
        host_seq: 0,
        state_hash: project.state_hash()?.bytes().to_vec(),
    };
    let view = CaptureView::new(&project, &rev, &id(2)?)?;
    assert!(
        view.prepare(
            view.context(),
            id(10)?,
            Platform::Uia,
            0,
            1,
            BoundsSpace::CapturePixels,
            elements(),
            &NeverCancel
        )
        .is_err()
    );
    assert!(
        view.prepare(
            view.context(),
            id(10)?,
            Platform::AndroidAx,
            0,
            1,
            BoundsSpace::HostPhysical,
            elements(),
            &NeverCancel
        )
        .is_err()
    );
    let snapshot = view.prepare(
        view.context(),
        id(10)?,
        Platform::AndroidAx,
        0,
        1,
        BoundsSpace::CapturePixels,
        elements(),
        &NeverCancel,
    )?;
    assert_eq!(snapshot.uia_collection_within_target(), None);
    Ok(())
}

#[test]
fn duplicate_ids_missing_parents_cycles_and_depth_are_refused() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    for raw in [
        vec![element("a", None, [0.0; 4]), element("a", None, [0.0; 4])],
        vec![element("a", Some("missing"), [0.0; 4])],
        vec![
            element("a", Some("b"), [0.0; 4]),
            element("b", Some("a"), [0.0; 4]),
        ],
    ] {
        assert!(prepare(&view, 10, raw).is_err());
    }
    let raw = (0..MAX_DEPTH + 2)
        .map(|i| {
            element(
                &i.to_string(),
                if i == 0 { None } else { Some("unused") },
                [0.0; 4],
            )
        })
        .collect::<Vec<_>>();
    let mut raw = raw;
    for (i, element) in raw.iter_mut().enumerate().skip(1) {
        element.parent_local_id = Some((i - 1).to_string());
    }
    assert!(matches!(
        prepare(&view, 10, raw),
        Err(Error::Limit("parent depth"))
    ));
    assert!(host.project().semantic_snapshots.is_empty());
    Ok(())
}

#[test]
fn text_geometry_count_and_cancellation_admission_precede_publishing() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    for invalid in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(
            prepare(
                &view,
                10,
                vec![element("a", None, [0.0, 0.0, invalid, 1.0])]
            )
            .is_err()
        );
    }
    let mut valid = element("a", None, [0.0; 4]);
    valid.text = "🦀".repeat(200);
    assert!(prepare(&view, 10, vec![valid]).is_ok());
    let mut invalid = element("a", None, [0.0; 4]);
    invalid.text = "a".repeat(201);
    assert!(matches!(
        prepare(&view, 10, vec![invalid]),
        Err(Error::Limit(_))
    ));
    let excessive = (0..MAX_ELEMENTS + 1)
        .map(|i| element(&i.to_string(), None, [0.0; 4]))
        .collect();
    assert!(matches!(
        prepare(&view, 10, excessive),
        Err(Error::Limit("elements"))
    ));
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(
        view.prepare(
            view.context(),
            id(10)?,
            Platform::Uia,
            0,
            1,
            BoundsSpace::CapturePixels,
            elements(),
            &cancelled
        ),
        Err(Error::Cancelled)
    ));
    let snapshot = prepare(&view, 10, elements())?;
    assert!(matches!(snapshot.encode(&cancelled), Err(Error::Cancelled)));
    assert!(
        view.plan(
            &snapshot,
            EditMetadata {
                lamport: u64::MAX,
                ..meta(1)?
            },
            &NeverCancel
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn collection_order_does_not_change_encoding_or_mutate_input_snapshots() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    let first = prepare(&view, 10, elements())?;
    let mut reverse = elements();
    reverse.reverse();
    let second = prepare(&view, 10, reverse)?;
    assert_eq!(first.encode(&NeverCancel)?, second.encode(&NeverCancel)?);
    let mut owned = first.elements()[0].clone();
    owned.name = "changed clone".into();
    assert_ne!(first.elements()[0].name, owned.name);
    Ok(())
}

#[test]
fn aggregate_text_and_wrong_visible_hash_fail_before_plan_creation() -> TestResult {
    let host = fixture()?;
    let view = view(&host)?;
    let raw = (0..520)
        .map(|i| {
            let mut element = element(&i.to_string(), None, [0.0; 4]);
            element.name = "a".repeat(4096);
            element
        })
        .collect();
    assert!(matches!(
        prepare(&view, 10, raw),
        Err(Error::Limit("total text"))
    ));
    let mut revision = host.revision()?;
    revision.state_hash[0] ^= 1;
    assert!(matches!(
        CaptureView::new(host.project(), &revision, &id(2)?),
        Err(Error::Stale)
    ));
    Ok(())
}

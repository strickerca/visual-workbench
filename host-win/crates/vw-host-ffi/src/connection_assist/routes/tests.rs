use super::*;
fn row(index: u32, metric: u32) -> RouteRow {
    RouteRow {
        family: RouteFamily::Ipv4,
        index,
        luid: u64::from(index) * 10,
        route_metric: 0,
        metric,
        automatic: true,
        next_hop_hash: [index as u8; 32],
    }
}
fn baseline() -> Vec<RouteRow> {
    vec![row(7, 5), row(2, 25)]
}
fn changed(plan: &Plan, before: &[RouteRow]) -> Vec<RouteRow> {
    before
        .iter()
        .cloned()
        .map(|mut r| {
            if plan.matches(&r) {
                r.automatic = plan.new_automatic;
                r.metric = plan.new_metric;
            }
            r
        })
        .collect()
}
#[test]
fn fix_demotes_selected_tether_without_modifying_an_alternate() -> Result<()> {
    let before = baseline();
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    assert_eq!(plan.new_metric, 500);
    assert!(!plan.new_automatic);
    assert_eq!(plan.old_metric, 5);
    let after = changed(&plan, &before);
    assert_eq!(after[1], before[1]);
    assert!(after[0].effective() > after[1].effective());
    assert!(preflight(&plan, &before).is_ok());
    Ok(())
}
#[test]
fn effective_route_metric_and_bounds_are_included() -> Result<()> {
    let mut before = baseline();
    before[1].route_metric = 1000;
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    assert_eq!(plan.new_metric, 1075);
    before[1].route_metric = u32::MAX;
    assert!(prepare(&before, 7, RouteFamily::Ipv4).is_err());
    Ok(())
}
#[test]
fn only_default_and_already_safe_interfaces_have_no_fix() {
    assert!(matches!(
        prepare(&[row(7, 5)], 7, RouteFamily::Ipv4),
        Err(Error::NoMetricFix)
    ));
    assert!(matches!(
        prepare(&baseline(), 2, RouteFamily::Ipv4),
        Err(Error::NoMetricFix)
    ));
    assert!(matches!(
        prepare(&baseline(), 7, RouteFamily::Ipv6),
        Err(Error::NoMetricFix)
    ));
}
#[test]
fn changed_metric_route_identity_automatic_mode_or_alternate_is_stale() -> Result<()> {
    let before = baseline();
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    for branch in 0..6 {
        let mut rows = before.clone();
        match branch {
            0 => rows[0].metric += 1,
            1 => rows[0].luid += 1,
            2 => rows[0].automatic = false,
            3 => rows[0].next_hop_hash[0] += 1,
            4 => rows[1].route_metric += 1,
            _ => {
                rows.pop();
            }
        }
        assert_eq!(preflight(&plan, &rows), Err(Error::StaleRoute));
    }
    Ok(())
}
#[test]
fn exact_snapshot_hash_is_order_independent_and_duplicate_sensitive() -> Result<()> {
    let original = baseline();
    let mut reordered = original.clone();
    reordered.reverse();
    assert_eq!(snapshot_hash(&original)?, snapshot_hash(&reordered)?);
    reordered.push(row(7, 5));
    assert_ne!(snapshot_hash(&original)?, snapshot_hash(&reordered)?);
    Ok(())
}
#[test]
fn revert_restores_exact_original_manual_values() -> Result<()> {
    let mut before = baseline();
    before[0].automatic = false;
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    let after = changed(&plan, &before);
    let inverse = verified_inverse(&plan, &before, &after)?;
    assert_eq!(inverse.kind, MetricActionKind::Revert);
    assert!(!inverse.new_automatic);
    assert_eq!(inverse.new_metric, 5);
    assert!(preflight(&inverse, &after).is_ok());
    assert_eq!(changed(&inverse, &after), before);
    Ok(())
}
#[test]
fn automatic_revert_admits_the_os_recomputed_metric_and_binds_its_actual_value() -> Result<()> {
    let before = baseline();
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    let after = changed(&plan, &before);
    let inverse = verified_inverse(&plan, &before, &after)?;
    assert!(inverse.new_automatic);
    let mut restored = changed(&inverse, &after);
    restored[0].metric = 12;
    let redo = verified_inverse(&inverse, &after, &restored)?;
    assert_eq!(redo.old_metric, 12);
    assert!(redo.old_automatic);
    Ok(())
}
#[test]
fn later_external_changes_refuse_the_retained_revert() -> Result<()> {
    let before = baseline();
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    let mut after = changed(&plan, &before);
    let inverse = verified_inverse(&plan, &before, &after)?;
    after[1].metric += 1;
    assert_eq!(preflight(&inverse, &after), Err(Error::StaleRoute));
    Ok(())
}
#[test]
fn an_unverified_or_partial_change_never_produces_a_revert_receipt() -> Result<()> {
    let before = baseline();
    let plan = prepare(&before, 7, RouteFamily::Ipv4)?;
    assert!(matches!(
        verified_inverse(&plan, &before, &before),
        Err(Error::MetricOutcomeUnknown)
    ));
    let mut after = changed(&plan, &before);
    after[1].metric += 1;
    assert!(matches!(
        verified_inverse(&plan, &before, &after),
        Err(Error::MetricOutcomeUnknown)
    ));
    Ok(())
}
#[test]
fn route_bounds_and_inconsistent_interface_fields_fail_before_action() {
    let mut rows = baseline();
    rows.push(row(7, 6));
    assert!(matches!(
        prepare(&rows, 7, RouteFamily::Ipv4),
        Err(Error::RouteUnavailable)
    ));
    assert!(snapshot_hash(&vec![row(1, 1); 4097]).is_err());
    assert!(snapshot_hash(&[row(0, 1)]).is_err());
    assert!(snapshot_hash(&[row(1, 10_000)]).is_err());
}

#[test]
fn only_proven_pre_setter_refusals_rearm_the_exact_action() -> Result<()> {
    let plan = prepare(&baseline(), 7, RouteFamily::Ipv4)?;
    for error in [Error::ElevationDeclined, Error::Busy, Error::Cancelled] {
        let action = RouteMetricAction::new(plan.clone());
        assert!(!action.used.swap(true, Ordering::AcqRel));
        action.release_known_refusal(&error);
        assert!(!action.used.swap(true, Ordering::AcqRel));
        assert_eq!(action.plan.snapshot_hash, plan.snapshot_hash);
        assert_eq!(action.plan.old_metric, plan.old_metric);
    }
    for error in [
        Error::MetricOutcomeUnknown,
        Error::StaleRoute,
        Error::MetricFailed,
        Error::WorkerUnavailable,
    ] {
        let action = RouteMetricAction::new(plan.clone());
        action.used.store(true, Ordering::Release);
        action.release_known_refusal(&error);
        assert!(action.used.load(Ordering::Acquire));
    }
    Ok(())
}

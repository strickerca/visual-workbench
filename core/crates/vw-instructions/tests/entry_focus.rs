mod common;
use common::*;
use vw_instructions::*;

#[test]
fn partial_entry_results_are_bounded_monotonic_literal_and_not_transactions() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let original = host.project().state_hash()?;
    let token = id(500)?;
    let mut entry = EntrySession::begin(
        &view(&host)?,
        &id(101)?,
        token.clone(),
        7,
        EntryMethod::Voice,
    )?;
    assert_eq!(entry.update(&token, 7, 1, "partial")?, EntryUpdate::Applied);
    assert_eq!(
        entry.update(&token, 7, 1, "partial")?,
        EntryUpdate::Duplicate
    );
    assert!(matches!(
        entry.update(&token, 7, 1, "changed same sequence"),
        Err(Error::Invalid(_))
    ));
    assert_eq!(entry.update(&token, 7, 0, "older")?, EntryUpdate::Stale);
    assert!(matches!(
        entry.update(&token, 7, 2, &"x".repeat(MAX_TEXT_BYTES + 1)),
        Err(Error::Limit(_))
    ));
    assert_eq!(entry.draft(), "partial");
    let final_text = "  final\n日本語 `literal`  ";
    entry.update(&token, 7, 2, final_text)?;
    assert_eq!(host.project().state_hash()?, original);
    let plan = entry.commit(&token, 7, &view(&host)?, meta(2)?)?;
    assert_eq!(host.project().state_hash()?, original);
    assert!(matches!(
        entry.update(&token, 7, 3, "late"),
        Err(Error::Closed)
    ));
    plan.submit(&mut host, &device(1), 4000)?;
    let stored = &host
        .project()
        .instructions
        .get(&id(101)?)
        .ok_or("instruction")?
        .definition;
    assert_eq!(stored.text, final_text);
    assert_eq!(stored.entry_method, "voice");
    Ok(())
}

#[test]
fn stale_entry_preserves_draft_but_focus_change_or_cancel_cannot_edit_another_field() -> TestResult
{
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let token = id(500)?;
    let mut entry = EntrySession::begin(
        &view(&host)?,
        &id(101)?,
        token.clone(),
        7,
        EntryMethod::Handwriting,
    )?;
    entry.update(&token, 7, 0, "retained handwritten draft")?;
    place(&mut host, 2)?;
    assert!(matches!(
        entry.commit(&token, 7, &view(&host)?, meta(3)?),
        Err(Error::Stale)
    ));
    assert_eq!(entry.draft(), "retained handwritten draft");
    assert!(matches!(
        entry.update(&token, 8, 1, "wrong focus"),
        Err(Error::Closed)
    ));
    assert!(matches!(
        entry.update(&id(501)?, 7, 1, "wrong session"),
        Err(Error::Closed)
    ));
    entry.cancel();
    assert_eq!(entry.draft(), "");
    assert!(matches!(
        entry.update(&token, 7, 2, "late callback"),
        Err(Error::Closed)
    ));
    assert_eq!(
        host.project()
            .instructions
            .get(&id(102)?)
            .ok_or("instruction")?
            .definition
            .text,
        "Instruction 2"
    );
    Ok(())
}

#[test]
fn focused_field_callbacks_keep_all_four_explicit_entry_methods() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    for (i, method) in [
        EntryMethod::PcKeyboard,
        EntryMethod::PhoneKeyboard,
        EntryMethod::Voice,
        EntryMethod::Handwriting,
    ]
    .into_iter()
    .enumerate()
    {
        let token = id(500 + i as u64)?;
        let mut entry =
            EntrySession::begin(&view(&host)?, &id(101)?, token.clone(), i as u64, method)?;
        entry.update(&token, i as u64, 1, "same literal, distinct provenance")?;
        assert_eq!(entry.method(), method);
        let plan = entry.commit(&token, i as u64, &view(&host)?, meta(10 + i as u64)?)?;
        plan.submit(&mut host, &device(1), 4000)?;
        assert_eq!(
            host.project()
                .instructions
                .get(&id(101)?)
                .ok_or("instruction")?
                .definition
                .entry_method,
            method.as_str()
        );
    }
    Ok(())
}

fn ticket(outcome: FocusOutcome) -> std::result::Result<FocusTicket, Box<dyn std::error::Error>> {
    if let FocusOutcome::Apply(ticket) = outcome {
        Ok(*ticket)
    } else {
        Err("expected applied focus".into())
    }
}

#[test]
fn focus_is_bound_to_peer_epoch_sequence_revision_and_late_ui_ticket() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    place(&mut host, 2)?;
    let v = view(&host)?;
    let epoch = id(700)?;
    let mut follower = FocusFollower::new(&v, device(2), epoch.clone());
    let one = FocusEvent::new(&v, device(2), epoch.clone(), 1, &id(11)?)?.to_json()?;
    let first = ticket(follower.receive(&one, &device(2), &v, 100)?)?;
    assert_eq!(first.instruction_id(), &id(101)?);
    assert!(first.within_local_target());
    assert!(follower.is_current(&first, &v, 110));
    assert!(matches!(
        follower.receive(&one, &device(2), &v, 111)?,
        FocusOutcome::Ignored
    ));
    let two = FocusEvent::new(&v, device(2), epoch, 2, &id(12)?)?.to_json()?;
    let second = ticket(follower.receive(&two, &device(2), &v, 120)?)?;
    assert!(!follower.is_current(&first, &v, 121));
    assert!(follower.is_current(&second, &v, 121));
    assert!(!follower.is_current(&second, &v, 120 + FOCUS_PENDING_TTL_MS + 1));
    assert!(matches!(
        follower.receive(&one, &device(2), &v, 122)?,
        FocusOutcome::Ignored
    ));
    follower.invalidate()?;
    assert!(!follower.is_current(&second, &v, 123));
    Ok(())
}

#[test]
fn future_revision_focus_waits_for_exact_ops_state_and_coalesces_to_latest() -> TestResult {
    let mut host = fixture()?;
    let before_project = host.project().clone();
    let before_revision = host.revision()?;
    let before = DocumentView::new(&before_project, &before_revision, &id(2)?)?;
    let epoch = id(700)?;
    let mut follower = FocusFollower::new(&before, device(2), epoch.clone());
    place(&mut host, 1)?;
    let after_one =
        FocusEvent::new(&view(&host)?, device(2), epoch.clone(), 1, &id(11)?)?.to_json()?;
    place(&mut host, 2)?;
    let after_two = FocusEvent::new(&view(&host)?, device(2), epoch, 2, &id(12)?)?.to_json()?;
    assert!(matches!(
        follower.receive(&after_one, &device(2), &before, 100)?,
        FocusOutcome::Deferred
    ));
    assert!(matches!(
        follower.receive(&after_two, &device(2), &before, 110)?,
        FocusOutcome::Deferred
    ));
    let ready = ticket(follower.poll(&view(&host)?, 350)?)?;
    assert_eq!(ready.marker_id(), &id(12)?);
    assert_eq!(ready.delivery_elapsed_ms, 240);
    assert!(!ready.within_local_target());
    assert!(follower.is_current(&ready, &view(&host)?, 350));
    Ok(())
}

#[test]
fn malformed_foreign_replayed_and_reconnected_focus_do_not_gain_admission() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let v = view(&host)?;
    let epoch = id(700)?;
    let mut follower = FocusFollower::new(&v, device(2), epoch.clone());
    let bytes = FocusEvent::new(&v, device(2), epoch.clone(), 1, &id(11)?)?.to_json()?;
    assert!(matches!(
        follower.receive(&bytes, &device(3), &v, 100),
        Err(Error::Unauthorized)
    ));
    let foreign = FocusEvent::new(&v, device(2), id(701)?, 1, &id(11)?)?.to_json()?;
    assert!(matches!(
        follower.receive(&foreign, &device(2), &v, 100),
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        follower.receive(&vec![b' '; MAX_FOCUS_BYTES + 1], &device(2), &v, 100),
        Err(Error::Limit(_))
    ));
    let mut malformed: serde_json::Value = serde_json::from_slice(&bytes)?;
    malformed["unknown"] = true.into();
    assert!(matches!(
        follower.receive(&serde_json::to_vec(&malformed)?, &device(2), &v, 100),
        Err(Error::Invalid(_))
    ));
    let accepted = ticket(follower.receive(&bytes, &device(2), &v, 100)?)?;
    let mut reused: serde_json::Value = serde_json::from_slice(&bytes)?;
    reused["instruction_id"] = id(102)?.to_string().into();
    assert!(matches!(
        follower.receive(&serde_json::to_vec(&reused)?, &device(2), &v, 101),
        Err(Error::Invalid(_))
    ));
    assert!(follower.is_current(&accepted, &v, 102));
    let mut reconnected = FocusFollower::new(&v, device(2), id(702)?);
    assert!(matches!(
        reconnected.receive(&bytes, &device(2), &v, 200),
        Err(Error::Unauthorized)
    ));
    follower.set_enabled(false)?;
    assert!(!follower.is_current(&accepted, &v, 200));
    Ok(())
}

#[test]
fn unmatched_focus_expires_and_a_newer_revision_never_reopens_stale_focus() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let old_project = host.project().clone();
    let old_revision = host.revision()?;
    let old = DocumentView::new(&old_project, &old_revision, &id(2)?)?;
    let mut follower = FocusFollower::new(&old, device(2), id(700)?);
    let bytes = FocusEvent::new(&old, device(2), id(700)?, 1, &id(11)?)?.to_json()?;
    let mut future: serde_json::Value = serde_json::from_slice(&bytes)?;
    future["source"]["host_seq"] = (old.binding().host_seq + 1).into();
    assert!(matches!(
        follower.receive(&serde_json::to_vec(&future)?, &device(2), &old, 100)?,
        FocusOutcome::Deferred
    ));
    assert!(matches!(
        follower.poll(&old, 100 + FOCUS_PENDING_TTL_MS + 1)?,
        FocusOutcome::Expired
    ));
    assert!(matches!(follower.poll(&old, 99), Err(Error::Invalid(_))));
    place(&mut host, 2)?;
    let fresh = view(&host)?;
    let old_event = FocusEvent::new(&old, device(2), id(700)?, 2, &id(11)?)?.to_json()?;
    assert!(matches!(
        follower.receive(&old_event, &device(2), &fresh, 2000)?,
        FocusOutcome::Ignored
    ));
    Ok(())
}

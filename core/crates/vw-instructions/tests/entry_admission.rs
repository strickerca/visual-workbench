//! The entry helper patch is intentionally applied only by the central owner.
mod common;
use common::*;
use vw_instructions::*;

#[test]
fn borrowed_preparation_refusal_keeps_editable_literal_then_seal_is_explicit() -> TestResult {
    let mut host = fixture()?;
    place(&mut host, 1)?;
    let token = id(500)?;
    let mut entry = EntrySession::begin(
        &view(&host)?,
        &id(101)?,
        token.clone(),
        7,
        EntryMethod::Voice,
    )?;
    entry.update(&token, 7, 1, "retained before admission")?;
    let before = host.checkpoint_bytes()?;
    let refused = entry.prepare(&token, 7, &view(&host)?, meta(2)?)?;
    // The adapter can reject this transaction's measured workspace or cancel
    // before publishing a handle without consuming the recognition draft.
    drop(refused);
    entry.update(&token, 7, 2, "edited after refusal")?;
    let plan = entry.prepare(&token, 7, &view(&host)?, meta(2)?)?;
    assert_eq!(host.checkpoint_bytes()?, before);
    entry.seal(&token, 7)?;
    assert_eq!(entry.draft(), "edited after refusal");
    assert!(matches!(
        entry.update(&token, 7, 3, "late"),
        Err(Error::Closed)
    ));
    plan.submit(&mut host, &device(1), 4000)?;
    assert_eq!(
        host.project()
            .instructions
            .get(&id(101)?)
            .ok_or("instruction")?
            .definition
            .text,
        "edited after refusal"
    );
    Ok(())
}
#[test]
fn wrong_focus_or_stale_prepare_never_seals_and_existing_commit_still_seals() -> TestResult {
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
    assert!(matches!(
        entry.prepare(&token, 8, &view(&host)?, meta(2)?),
        Err(Error::Closed)
    ));
    assert!(matches!(entry.seal(&id(501)?, 7), Err(Error::Closed)));
    entry.update(&token, 7, 1, "still active")?;
    let plan = entry.commit(&token, 7, &view(&host)?, meta(2)?)?;
    assert!(matches!(
        entry.prepare(&token, 7, &view(&host)?, meta(2)?),
        Err(Error::Closed)
    ));
    plan.submit(&mut host, &device(1), 4000)?;
    let mut stale = EntrySession::begin(&view(&host)?, &id(101)?, id(502)?, 9, EntryMethod::Voice)?;
    place(&mut host, 3)?;
    assert!(matches!(
        stale.prepare(&id(502)?, 9, &view(&host)?, meta(4)?),
        Err(Error::Stale)
    ));
    stale.update(&id(502)?, 9, 1, "retained stale literal")?;
    assert_eq!(stale.draft(), "retained stale literal");
    Ok(())
}

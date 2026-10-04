mod common;
use common::*;
use vw_ai::{budget::*, *};
use vw_mask::{Mask, MaskError, Rect, Size};

fn sparse_mask(width: u32, height: u32) -> std::result::Result<Mask, MaskError> {
    Mask::rectangle(
        Size::new(width, height)?,
        Rect {
            x: 100.0,
            y: 100.0,
            width: 1.0,
            height: 1.0,
        },
    )
}

#[derive(Default)]
struct NeverCalledProvider {
    attempts: usize,
}
impl ProviderAdapter for NeverCalledProvider {
    fn send_once(
        &mut self,
        _request: AuthorizedRequest,
        _cancel: &dyn Cancellation,
    ) -> Result<ProviderResponse> {
        self.attempts += 1;
        Err(Error::Unsupported)
    }
}

fn assert_rejected_before_attempt(preparation: Result<Prepared>, expected: Error) -> TestResult {
    let root = temp()?;
    let path = root.path().join("budget.sqlite");
    let mut ledger = SqliteBudgetLedger::create(&path)?;
    let mut provider = NeverCalledProvider::default();
    let result = preparation.and_then(|prepared| {
        ledger.reserve(
            prepared.review(),
            Confirmation::explicit_send(prepared.review(), 20000, false)?,
            BudgetPolicy::default(),
        )?;
        let permit = ledger.begin_attempt(prepared.review())?;
        let request = prepared.authorize(permit, &NeverCancel)?;
        provider.send_once(request, &NeverCancel).map(|_| ())
    });
    assert_eq!(result, Err(expected));
    assert_eq!(provider.attempts, 0);
    assert_eq!(ledger.spent_on(20000)?, 0);
    drop(ledger);
    let connection = rusqlite::Connection::open(&path)?;
    let entries: i64 =
        connection.query_row("SELECT count(*) FROM entries", [], |row| row.get(0))?;
    assert_eq!(entries, 0, "no reservation or attempt may be persisted");
    Ok(())
}

#[test]
fn nonallocating_admission_uses_the_exact_dense_work_boundary() -> TestResult {
    // One sparse tile; these calls must not allocate full-document buffers or
    // perform the billion-operation morphology just to decide admission.
    let mask = sparse_mask(2000, 1500)?;
    let version = mask.version();
    assert_eq!(mask.tile_count(), 1);
    assert_eq!(mask.admit_morphology(55), Ok(())); // 999,000,000 charged work
    assert_eq!(mask.admit_morphology(56), Err(MaskError::Limit));
    assert_eq!(mask.admit_morphology(64), Err(MaskError::Limit));
    assert_eq!(mask.version(), version);
    assert_eq!(mask.tile_count(), 1);
    assert_eq!(mask.admit_morphology(0), Ok(()));
    assert_eq!(Mask::empty(mask.size()).admit_morphology(64), Ok(()));
    assert_eq!(
        Mask::empty(mask.size()).admit_morphology(257),
        Err(MaskError::Limit)
    );
    let exhausted = Mask::from_tiles(mask.size(), u64::MAX, vec![])?;
    assert_eq!(
        exhausted.admit_morphology(0),
        Err(MaskError::VersionExhausted)
    );
    Ok(())
}

#[test]
fn feather_and_proof_only_work_limits_reject_before_reservation_or_provider_attempt() -> TestResult
{
    let source = png8(2000, 1500, |_, _| [20, 40, 60, 255])?;
    let mask = sparse_mask(2000, 1500)?;
    for (radius, error) in [
        (64, Error::Limit("mask feather work")),
        (55, Error::Limit("exterior proof dilation work")),
    ] {
        let mut opts = options(1)?;
        opts.feather_px = radius;
        assert_rejected_before_attempt(
            Prepared::new(&source, std::slice::from_ref(&mask), opts, &NeverCancel),
            error,
        )?;
    }
    Ok(())
}

#[test]
fn affordable_preparation_with_unaffordable_completion_never_offers_a_paid_request() -> TestResult {
    let source = png8(512, 512, |_, _| [20, 40, 60, 255])?;
    let mask = sparse_mask(512, 512)?;
    // The same valid pixels/profile/mask complete preparation with the default
    // budget. At 100 MiB preparation used to fit while completion could not.
    drop(Prepared::new(
        &source,
        std::slice::from_ref(&mask),
        options(1)?,
        &NeverCancel,
    )?);
    let mut opts = options(1)?;
    opts.limits.memory_bytes = 100 * 1024 * 1024;
    assert_rejected_before_attempt(
        Prepared::new(&source, &[mask], opts, &NeverCancel),
        Error::Limit("working memory; use a smaller region or a tiled adapter"),
    )
}

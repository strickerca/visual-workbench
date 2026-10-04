use super::*;
use crate::{ProjectSession, worker::Worker};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use vw_ai::{Completed, Prepared};
use vw_mask::{Mask, Size};
use vw_model::Id;
pub(super) struct RequestState {
    pub prepared: Prepared,
    pub options: AiPrepareOptions,
    pub source: vw_proto::v1::AddAsset,
    pub configuration: configuration::Configuration,
    pub base: Option<Completed>,
    pub partial: Option<Completed>,
    pub acceptance: Option<Mask>,
    pub raw: Arc<Vec<u8>>,
    pub raw_asset: Option<vw_proto::v1::AddAsset>,
    pub candidate_id: String,
    pub settlement: AiSettlement,
    pub send_consumed: bool,
    pub accepted_brush_ids: std::collections::BTreeSet<Id>,
    pub save: Option<Arc<result::SavePayload>>,
    _permit: prepare::RequestPermit,
}
impl RequestState {
    pub fn current(&self) -> AiEditResult<&Completed> {
        self.partial
            .as_ref()
            .or(self.base.as_ref())
            .ok_or(AiEditError::Invalid)
    }
    pub fn info(&self) -> AiEditResult<AiCandidateInfo> {
        let image = self.current()?;
        Ok(AiCandidateInfo {
            request_id: image.request_id().into(),
            candidate_id: self.candidate_id.clone(),
            binding: self.options.binding.clone(),
            width: image.image().width(),
            height: image.image().height(),
            bit_depth: image.image().bit_depth(),
            partial: self.partial.is_some(),
            settlement: self.settlement,
            actual_microusd: image.actual_microusd(),
            proof: proof(image.proof()),
        })
    }
    pub fn resident(&self) -> u64 {
        let p =
            u64::from(self.prepared.source().width()) * u64::from(self.prepared.source().height());
        p * 32
            + self.raw.len() as u64
            + self.prepared.request_image_png().len() as u64
            + self.prepared.request_mask_png().len() as u64
            + 8 * 1024 * 1024
    }
}
pub(super) fn proof(p: &vw_ai::proof::Proof) -> AiProofInfo {
    AiProofInfo {
        changed_outside: p.changed_outside,
        outside_sha256_before: p.outside_sha256_before.clone(),
        outside_sha256_after: p.outside_sha256_after.clone(),
        changed_unaccepted: p.acceptance.as_ref().map(|a| a.changed_unaccepted),
        unaccepted_sha256_before: p.acceptance.as_ref().map(|a| a.before_sha256.clone()),
        unaccepted_sha256_after: p.acceptance.as_ref().map(|a| a.after_sha256.clone()),
        delta_e2000_mean: p.metrics.delta_e2000_mean,
        delta_e2000_max: p.metrics.delta_e2000_max,
        ssim_inside_mask: p.metrics.ssim_inside_mask,
    }
}
#[derive(uniffi::Object)]
pub struct AiRequest {
    pub(super) worker: Worker<RequestState>,
    pub(super) project: Arc<ProjectSession>,
    service: Arc<AiService>,
    closed: Arc<AtomicBool>,
    review: AiReview,
    pub(super) save_admission: admission::SaveAdmission,
}
impl AiRequest {
    pub(super) fn new(
        project: Arc<ProjectSession>,
        service: Arc<AiService>,
        prepared: Prepared,
        options: AiPrepareOptions,
        source: vw_proto::v1::AddAsset,
        configuration: configuration::Configuration,
        permit: prepare::RequestPermit,
    ) -> AiEditResult<Arc<Self>> {
        let save_admission =
            admission::SaveAdmission::new(&prepared, &source, &options, &configuration)?;
        let r = prepared.review();
        let crop = r.crop();
        let review = AiReview {
            request_id: r.request_id().into(),
            binding: options.binding.clone(),
            source_asset_id: source.asset_id.clone(),
            source_file_sha256: r.source_file_sha256().into(),
            mask_sha256: r.mask_sha256().into(),
            configuration_fingerprint: options.configuration_fingerprint.clone(),
            model: r.model().into(),
            quality: r.quality().into(),
            source_width: r.source_width(),
            source_height: r.source_height(),
            source_bit_depth: r.source_bit_depth(),
            model_width: crop.model_width,
            model_height: crop.model_height,
            crop_x: crop.crop.x,
            crop_y: crop.crop.y,
            crop_width: crop.crop.width,
            crop_height: crop.crop.height,
            feather_px: r.feather_px(),
            estimated_microusd: r.estimated_microusd(),
            estimate_provenance: options.estimate.as_ref().map(|e| e.provenance.clone()),
            estimate_expires_on: options.estimate.as_ref().map(|e| e.expires_on.clone()),
            configuration_expires_on: configuration.expires_on.clone(),
            provider_copy_reduces_depth: r.source_bit_depth() == 16,
        };
        let state = RequestState {
            prepared,
            options,
            source,
            configuration,
            base: None,
            partial: None,
            acceptance: None,
            raw: Arc::new(vec![]),
            raw_asset: None,
            candidate_id: String::new(),
            settlement: AiSettlement::UsageMissing,
            send_consumed: false,
            accepted_brush_ids: std::collections::BTreeSet::new(),
            save: None,
            _permit: permit,
        };
        Ok(Arc::new(Self {
            worker: Worker::new("vw-ai-request", state, 2)?,
            project,
            service,
            closed: Arc::new(AtomicBool::new(false)),
            review,
            save_admission,
        }))
    }
    fn live(&self, cancel: Arc<crate::Cancellation>) -> Stop {
        Stop {
            cancel,
            request: self.closed.clone(),
            service: self.service.closed.clone(),
            project: self.project.closed.clone(),
        }
    }
}
pub(super) struct Stop {
    cancel: Arc<crate::Cancellation>,
    request: Arc<AtomicBool>,
    service: Arc<AtomicBool>,
    project: Arc<AtomicBool>,
}
impl vw_ai::Cancellation for Stop {
    fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
            || self.request.load(Ordering::Acquire)
            || self.service.load(Ordering::Acquire)
            || self.project.load(Ordering::Acquire)
    }
}
impl Stop {
    pub fn check(&self) -> AiEditResult<()> {
        if vw_ai::Cancellation::is_cancelled(self) {
            Err(AiEditError::Cancelled)
        } else {
            Ok(())
        }
    }
}
#[uniffi::export]
impl AiRequest {
    pub fn describe(&self) -> AiReview {
        self.review.clone()
    }
    pub async fn send(
        &self,
        request_id: String,
        displayed_estimate_microusd: u64,
        acknowledge_soft_budget: bool,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiCandidateInfo> {
        let stop = self.live(cancellation.clone());
        stop.check()?;
        let expected = self.review.binding.clone();
        let admission = self.save_admission;
        let project_stop = self.live(cancellation.clone());
        self.project
            .worker
            .call(move |state| {
                Ok((|| {
                    project_stop.check()?;
                    admission.check(state)?;
                    crate::workflow::check_binding(state, &expected)?;
                    Ok::<(), AiEditError>(())
                })())
            })
            .await??;
        let shared = self.service.state.clone();
        self.worker
            .call(move |state| {
                Ok((|| {
                    stop.check()?;
                    if state.send_consumed {
                        return Err(AiEditError::AttemptConsumed);
                    }
                    if request_id != state.prepared.review().request_id()
                        || state.prepared.review().estimated_microusd()
                            != Some(displayed_estimate_microusd)
                        || displayed_estimate_microusd == 0
                    {
                        return Err(AiEditError::Estimate);
                    }
                    let today = configuration::today()?;
                    let mut owner = shared.try_lock().map_err(|_| AiEditError::Busy)?;
                    if owner.configuration.fingerprint()? != state.options.configuration_fingerprint
                    {
                        return Err(AiEditError::Stale);
                    }
                    if !owner.configuration.current(today)? {
                        return Err(AiEditError::Estimate);
                    }
                    configuration::estimate(
                        state
                            .options
                            .estimate
                            .as_ref()
                            .ok_or(AiEditError::Estimate)?,
                        today,
                    )?;
                    let confirmation = vw_ai::budget::Confirmation::explicit_send(
                        state.prepared.review(),
                        today,
                        acknowledge_soft_budget,
                    )?;
                    let policy = vw_ai::budget::BudgetPolicy {
                        daily_soft_limit_microusd: owner.configuration.daily_soft_budget_microusd,
                    };
                    let mut adapter = owner::adapter(&cancellation)?;
                    // Consume the handle before crossing the provider boundary. Ledger
                    // refuses uncertain attempts across fresh handles/process restarts.
                    state.send_consumed = true;
                    let outcome = match adapter.send_confirmed(
                        &state.prepared,
                        &mut owner.ledger,
                        confirmation,
                        policy,
                        &stop,
                    ) {
                        Ok(value) => value,
                        Err(vw_ai_provider::Error::Core(vw_ai::Error::SoftBudget)) => {
                            state.send_consumed = false;
                            return Err(AiEditError::SoftBudget);
                        }
                        Err(vw_ai_provider::Error::Busy) => {
                            state.send_consumed = false;
                            return Err(AiEditError::Busy);
                        }
                        Err(error) => return Err(error.into()),
                    };
                    state.settlement = match outcome.settlement {
                        vw_ai_provider::Settlement::UsagePriced { .. } => AiSettlement::UsagePriced,
                        vw_ai_provider::Settlement::UsageMissing => AiSettlement::UsageMissing,
                        vw_ai_provider::Settlement::LedgerUnavailable => {
                            AiSettlement::LedgerUnavailable
                        }
                    };
                    drop(owner);
                    state.raw = Arc::new(outcome.response.image_png().to_vec());
                    let model = state.prepared.review().crop();
                    memory(
                        state.resident()
                            + outcome.response.image_png().len() as u64
                            + u64::from(model.model_width) * u64::from(model.model_height) * 32
                            + 32 * 1024 * 1024,
                        state.options.memory_budget_bytes,
                    )?;
                    let raw = vw_raster::decode(
                        state.raw.as_slice(),
                        vw_raster::DecodeLimits {
                            max_encoded_bytes: vw_ai::MAX_ENCODED,
                            max_pixels: vw_ai::MAX_PIXELS,
                            max_memory_bytes: state.options.memory_budget_bytes
                                - state.resident()
                                - outcome.response.image_png().len() as u64,
                        },
                    )?;
                    if raw
                        .icc
                        .as_ref()
                        .is_some_and(|p| p.len() > admission::MAX_RESPONSE_ICC)
                    {
                        return Err(AiEditError::Limit);
                    }
                    state.raw_asset = Some(result::asset(
                        &raw,
                        &vw_model::AssetId::hash(state.raw.as_slice()),
                        state.raw.len() as u64,
                    ));
                    drop(raw);
                    let completed = state.prepared.finish(outcome.response, &stop)?;
                    if completed.proof().changed_outside != 0 {
                        return Err(AiEditError::Proof);
                    }
                    state.candidate_id = completed.proof().result_pixels_sha256.clone();
                    state.base = Some(completed);
                    state.info()
                })())
            })
            .await?
    }
    pub async fn candidate(
        &self,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiCandidateInfo> {
        let stop = self.live(cancellation);
        self.worker
            .call(move |s| {
                Ok((|| {
                    stop.check()?;
                    s.info()
                })())
            })
            .await?
    }
    pub async fn compare(
        &self,
        candidate_id: String,
        mode: AiCompareMode,
        region: AiRegion,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiPixels> {
        let stop = self.live(cancellation);
        self.worker
            .call(move |s| {
                Ok((|| {
                    stop.check()?;
                    if candidate_id != s.candidate_id {
                        return Err(AiEditError::Stale);
                    }
                    pixels::compare(s, mode, region, &stop)
                })())
            })
            .await?
    }
    pub async fn accept_brush(
        &self,
        brush: AiAcceptanceBrush,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiCandidateInfo> {
        let next_id = Id::try_from(brush.next_candidate_id.clone())?;
        if brush.points.is_empty()
            || brush.points.len() > vw_mask::MAX_PATH_POINTS
            || !brush.radius.is_finite()
            || brush.radius <= 0.0
        {
            return Err(AiEditError::Invalid);
        }
        let stop = self.live(cancellation);
        self.worker
            .call(move |s| {
                Ok((|| {
                    stop.check()?;
                    if brush.expected_candidate_id != s.candidate_id
                        || brush.next_candidate_id == s.candidate_id
                        || s.save.is_some()
                        || s.accepted_brush_ids.contains(&next_id)
                    {
                        return Err(AiEditError::Stale);
                    }
                    if s.accepted_brush_ids.len() >= 4096 {
                        return Err(AiEditError::Limit);
                    }
                    let base = s.base.as_ref().ok_or(AiEditError::Invalid)?;
                    let size = Size::new(base.image().width(), base.image().height())?;
                    memory(
                        s.resident() + size.pixels() as u64 * 64 + 64 * 1024 * 1024,
                        s.options.memory_budget_bytes,
                    )?;
                    let points = brush
                        .points
                        .iter()
                        .map(|p| vw_mask::Point { x: p.x, y: p.y })
                        .collect::<Vec<_>>();
                    let mark = Mask::paint(size, &points, brush.radius, brush.opacity)?;
                    let start = if brush.clear_first {
                        Mask::empty(size)
                    } else {
                        s.acceptance.clone().unwrap_or_else(|| Mask::empty(size))
                    };
                    let acceptance = start.combine(
                        &mark,
                        if brush.subtract {
                            vw_mask::Combine::Subtract
                        } else {
                            vw_mask::Combine::Add
                        },
                    )?;
                    stop.check()?;
                    let partial = base.accept_part(&acceptance, &stop)?;
                    if partial.proof().changed_outside != 0
                        || partial
                            .proof()
                            .acceptance
                            .as_ref()
                            .is_none_or(|p| p.changed_unaccepted != 0)
                    {
                        return Err(AiEditError::Proof);
                    }
                    stop.check()?;
                    s.accepted_brush_ids.insert(next_id);
                    s.acceptance = Some(acceptance);
                    s.partial = Some(partial);
                    s.candidate_id = brush.next_candidate_id;
                    s.info()
                })())
            })
            .await?
    }
    pub async fn save(
        &self,
        candidate_id: String,
        options: AiSaveOptions,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<AiResultReceipt> {
        let stop = self.live(cancellation.clone());
        let stop2 = self.live(cancellation.clone());
        let payload = self
            .worker
            .call(move |s| {
                Ok((|| {
                    stop.check()?;
                    if s.candidate_id != candidate_id {
                        return Err(AiEditError::Stale);
                    }
                    result::prepare(s, options, &stop)
                })())
            })
            .await??;
        self.project
            .worker
            .call(move |state| Ok(result::save(state, &payload, &cancellation, &stop2)))
            .await?
    }
    pub async fn shutdown(&self) -> AiEditResult<()> {
        self.closed.store(true, Ordering::Release);
        self.worker.shutdown(|_| Ok(())).await?;
        Ok(())
    }
}
impl Drop for AiRequest {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}

use super::*;
use crate::{ProjectSession, worker::startup};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use vw_model::Id;
static REQUESTS: AtomicUsize = AtomicUsize::new(0);
pub(super) struct RequestPermit;
impl RequestPermit {
    pub(super) fn acquire() -> AiEditResult<Self> {
        REQUESTS
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n == 0).then_some(1)
            })
            .map_err(|_| AiEditError::Busy)?;
        Ok(Self)
    }
}
impl Drop for RequestPermit {
    fn drop(&mut self) {
        REQUESTS.fetch_sub(1, Ordering::AcqRel);
    }
}
#[uniffi::export]
impl ProjectSession {
    pub async fn prepare_ai_edit(
        self: Arc<Self>,
        service: Arc<AiService>,
        options: AiPrepareOptions,
        cancellation: Arc<crate::Cancellation>,
    ) -> AiEditResult<Arc<AiRequest>> {
        self.check_open()?;
        options.binding.validate()?;
        if options.instruction.len() > 16 * 1024
            || options.instruction.contains('\0')
            || options.selections.is_empty()
            || options.selections.len() > 64
            || options.feather_px > 64
        {
            return Err(AiEditError::Invalid);
        }
        memory(RAW_RESERVE + 32 * 1024 * 1024, options.memory_budget_bytes)?;
        let intent = Id::try_from(options.intent_id.clone())?;
        let permit = RequestPermit::acquire()?;
        let config = {
            let state = service.state.try_lock().map_err(|_| AiEditError::Busy)?;
            if state.configuration.fingerprint()? != options.configuration_fingerprint {
                return Err(AiEditError::Stale);
            }
            state.configuration.clone()
        };
        let today = configuration::today()?;
        let tokens = options
            .estimate
            .as_ref()
            .map(|e| configuration::estimate(e, today))
            .transpose()?;
        let owner_closed = service.closed.clone();
        check(&owner_closed, &cancellation)?;
        let project_closed = self.closed.clone();
        let expected = options.binding.clone();
        let versions = options.selections.clone();
        let budget = options.memory_budget_bytes;
        let cancel = cancellation.clone();
        let (input, mut instructions) = self
            .worker
            .call(move |state| {
                Ok((|| -> AiEditResult<_> {
                    check(&project_closed, &cancel)?;
                    crate::workflow::canonical_workspace(
                        state,
                        budget.min(crate::workflow::MAX_MEMORY),
                        true,
                    )?;
                    crate::workflow::check_binding(state, &expected)?;
                    let (_, instructions) = context::collect(state, &expected)?;
                    Ok((
                        crate::masks::ai_bridge::inputs(
                            state,
                            &expected,
                            &versions,
                            budget - RAW_RESERVE,
                            &cancel,
                        )?,
                        instructions,
                    ))
                })())
            })
            .await??;
        let project = self.clone();
        let cancel = cancellation.clone();
        let request = startup(move || {
            Ok((|| -> AiEditResult<_> {
                check(&owner_closed, &cancellation)?;
                check(&project.closed, &cancellation)?;
                if !options.instruction.trim().is_empty() {
                    instructions.push(vw_ai::Instruction {
                        role: vw_ai::InstructionRole::Change,
                        text: options.instruction.clone(),
                    });
                }
                let prepared = vw_ai::Prepared::new(
                    &input.source,
                    &input.masks,
                    vw_ai::PrepareOptions {
                        provider: config.provider.clone(),
                        revision: vw_ai::RevisionBinding {
                            project_id: Id::try_from(options.binding.project_id.clone())?,
                            host_seq: options.binding.host_seq,
                            state_hash: options.binding.state_hash.clone(),
                        },
                        intent_id: intent,
                        instructions,
                        feather_px: options.feather_px,
                        estimated_tokens: tokens,
                        source_policy: vw_ai::SourcePolicy {
                            assume_untagged_srgb: options.assume_untagged_srgb,
                            allow_16bit_provider_copy: options.allow_16bit_provider_copy,
                        },
                        limits: vw_ai::Limits {
                            memory_bytes: budget - RAW_RESERVE,
                        },
                    },
                    cancellation.as_ref(),
                )?;
                let (width, height) = if input.source_asset.orientation >= 5 {
                    (input.source_asset.height, input.source_asset.width)
                } else {
                    (input.source_asset.width, input.source_asset.height)
                };
                if prepared.source().width() != width
                    || prepared.source().height() != height
                    || u32::from(prepared.source().bit_depth()) != input.source_asset.bit_depth
                    || (!input.source_asset.icc_profile.is_empty()
                        && prepared.source().icc() != input.source_asset.icc_profile.as_slice())
                {
                    return Err(AiEditError::Proof);
                }
                // Full result, immutable base, optional replacement and private brush
                // are charged before offering Send, not first discovered after billing.
                memory(
                    (u64::from(width) * u64::from(height) * 112 + RAW_RESERVE + 96 * 1024 * 1024)
                        .max(
                            u64::from(width) * u64::from(height) * 74
                                + 2 * RAW_RESERVE
                                + 41 * 1024 * 1024,
                        )
                        + prepared.request_image_png().len() as u64
                        + prepared.request_mask_png().len() as u64,
                    budget,
                )?;
                // Metadata inspection briefly retains the response and its immutable
                // project copy together. Reserve that exact extra lifetime too.
                let model = prepared.review().crop();
                memory(
                    u64::from(width) * u64::from(height) * 32
                        + 2 * RAW_RESERVE
                        + u64::from(model.model_width) * u64::from(model.model_height) * 32
                        + 40 * 1024 * 1024
                        + prepared.request_image_png().len() as u64
                        + prepared.request_mask_png().len() as u64,
                    budget,
                )?;
                check(&owner_closed, &cancellation)?;
                check(&project.closed, &cancellation)?;
                AiRequest::new(
                    project,
                    service,
                    prepared,
                    options,
                    input.source_asset,
                    config,
                    permit,
                )
            })())
        })
        .await??;
        let admission = request.save_admission;
        let expected = request.describe().binding;
        let closed = self.closed.clone();
        let validated = self
            .worker
            .call(move |state| {
                Ok((|| -> AiEditResult<_> {
                    check(&closed, &cancel)?;
                    admission.check(state)?;
                    crate::workflow::check_binding(state, &expected)?;
                    Ok(())
                })())
            })
            .await;
        let failure = match validated {
            Ok(Ok(())) => None,
            Ok(Err(e)) => Some(e),
            Err(e) => Some(e.into()),
        };
        if let Some(error) = failure {
            request.shutdown().await?;
            return Err(error);
        }
        Ok(request)
    }
}

//! Before payment, reserve the complete known Result transaction and retained
//! request/output lifetime. Provider validity and later project edits still fail
//! independently; this is not a guarantee about an arbitrary paid response.
use super::*;
use crate::project::ProjectState;
use vw_ai::Prepared;
pub(super) const MAX_RESPONSE_ICC: usize = 4096;
const MAX_PROOF: usize = 16 * 1024;
const MAX_REQUEST: usize = 900_000;
#[derive(Clone, Copy)]
pub(super) struct SaveAdmission {
    pub resident: u64,
    pub transaction: u64,
    pub budget: u64,
}
fn add(a: u64, b: u64) -> AiEditResult<u64> {
    a.checked_add(b).ok_or(AiEditError::Limit)
}
fn mul(a: u64, b: u64) -> AiEditResult<u64> {
    a.checked_mul(b).ok_or(AiEditError::Limit)
}
pub(super) fn metadata_json(
    prepared: &Prepared,
    source_id: &str,
    configuration: &configuration::Configuration,
    provenance: Option<&str>,
    settlement: &str,
    actual: Option<u64>,
    origin: &str,
) -> AiEditResult<String> {
    let json=serde_json::to_string(&serde_json::json!({"schema":1,"request_id":prepared.review().request_id(),"source_asset_id":source_id,"review":prepared.review(),"instructions":prepared.instructions(),"configuration":configuration,"estimate_provenance":provenance,"settlement":settlement,"actual_microusd":actual,"origin":origin})).map_err(|_|AiEditError::Invalid)?;
    if json.len() > MAX_REQUEST {
        return Err(AiEditError::Limit);
    }
    Ok(json)
}
impl SaveAdmission {
    pub fn new(
        prepared: &Prepared,
        source: &vw_proto::v1::AddAsset,
        options: &AiPrepareOptions,
        configuration: &configuration::Configuration,
    ) -> AiEditResult<Self> {
        let request = metadata_json(
            prepared,
            &source.asset_id,
            configuration,
            options.estimate.as_ref().map(|e| e.provenance.as_str()),
            "ledger_unavailable",
            Some(u64::MAX),
            "provider_payload",
        )?;
        let pixels = mul(
            u64::from(prepared.source().width()),
            u64::from(prepared.source().height()),
        )?;
        Self::bound(
            pixels,
            prepared.source().icc().len() as u64,
            request.len() as u64,
            add(
                prepared.request_image_png().len() as u64,
                prepared.request_mask_png().len() as u64,
            )?,
            options.memory_budget_bytes,
        )
    }
    fn bound(
        pixels: u64,
        profile: u64,
        request: u64,
        request_pngs: u64,
        budget: u64,
    ) -> AiEditResult<Self> {
        if profile > 4 * 1024 * 1024 || request > MAX_REQUEST as u64 {
            return Err(AiEditError::Limit);
        }
        // Current request state (32 bytes/pixel), complete raw PNG, composite
        // Vec geometric-capacity slack (2*64 MiB), and private coverage PNG
        // lifetime (2 bytes/pixel+1 MiB), plus fixed ownership/profile overhead.
        let resident = add(
            add(add(mul(pixels, 34)?, mul(RAW_RESERVE, 3)?)?, request_pngs)?,
            add(mul(profile, 4)?, 9 * 1024 * 1024)?,
        )?;
        // Matches the reviewed borrowed WorkspaceEstimate upper-bound rules:
        // 4 bytes per JSON byte, worst 6-byte string escape; each ICC byte is
        // serialized as <=3 digits+comma and one 256-byte scalar node. 256 KiB
        // additionally covers all fixed fields/nodes of <=9 ordinary save ops.
        // Sixteen canonical transaction representations are required by Save.
        let strings = mul(add(request, MAX_PROOF as u64)?, 24)?;
        let profiles = mul(add(profile, MAX_RESPONSE_ICC as u64)?, 272)?;
        let transaction = mul(add(add(strings, profiles)?, 256 * 1024)?, 16)?;
        memory(add(resident, transaction)?, budget)?;
        Ok(Self {
            resident,
            transaction,
            budget,
        })
    }
    /// No JSON/Value/hash clone occurs before this borrowed state admission.
    pub fn check(self, state: &ProjectState) -> AiEditResult<()> {
        let remaining = self
            .budget
            .checked_sub(self.resident)
            .ok_or(AiEditError::Limit)?;
        crate::workflow::admit(
            state,
            remaining.min(crate::workflow::MAX_MEMORY),
            true,
            self.transaction,
        )?;
        Ok(())
    }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    #[test]
    fn known_save_envelope_is_checked_without_allocating_pixels_or_profile() {
        let small = SaveAdmission::bound(256, 600, 5000, 2000, 256 * 1024 * 1024).unwrap();
        assert!(small.resident > 3 * RAW_RESERVE);
        assert!(small.transaction > 16 * 1024 * 1024);
        assert!(SaveAdmission::bound(256, 4 * 1024 * 1024, 5000, 2000, MAX_MEMORY).is_err());
        assert!(SaveAdmission::bound(u64::MAX, 600, 5000, 2000, MAX_MEMORY).is_err());
    }
}

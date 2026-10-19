#[cfg(any(windows, test))]
use crate::{Error, Result};
use vw_assets::Manifest;
use vw_stream::MemoryCharge;
#[cfg(any(windows, test))]
use vw_stream::{MemoryKind, SessionBudget};

/// Hard Windows Job/process commit allowance, reserved in the parent's supplied
/// session before launch. It covers the entire child, including its codec and
/// output-verification allocations. This lower cap is unmeasured at runtime.
pub const WORKER_MEMORY_BYTES: u64 = 256 * 1024 * 1024;
/// Parent setup/hash buffers, path/argv storage, pipe and request JSON overhead.
#[cfg(windows)]
pub(crate) const PARENT_WORK_BYTES: u64 = 2 * 1024 * 1024;
/// Raw <=8MiB manifest, <=100k fixed-size records, pyramid and encoding overhead.
#[cfg(any(windows, test))]
pub(crate) const MANIFEST_OWNER_BYTES: u64 = 24 * 1024 * 1024;
#[cfg(windows)]
pub(crate) const DECODE_WORK_BYTES: u64 = 32 * 1024 * 1024;

/// A derived inventory and its retained session reservation. The manifest drops
/// before its charge. Keep this owner alive through every use or cache install;
/// cloning its borrowed Manifest requires a separate reservation by the caller.
pub struct BudgetedManifest {
    pub(crate) manifest: Manifest,
    pub(crate) charge: MemoryCharge,
}
impl BudgetedManifest {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn charge(&self) -> &MemoryCharge {
        &self.charge
    }
    /// Transfer both into one retained payload owner before any fallible work.
    /// Discarding the charge while retaining the Manifest violates admission.
    pub fn into_parts(self) -> (Manifest, MemoryCharge) {
        (self.manifest, self.charge)
    }
}

#[cfg(any(windows, test))]
pub(crate) fn reserve(
    budget: &SessionBudget,
    kind: MemoryKind,
    bytes: u64,
) -> Result<MemoryCharge> {
    budget.reserve(kind, bytes).map_err(|_| Error::Memory {
        estimated: budget.used_bytes().saturating_add(bytes),
        budget: budget.limit_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vw_assets::{Source, TileRecord};

    #[test]
    fn worker_admission_uses_the_callers_existing_pool() -> Result<()> {
        let budget = SessionBudget::new(WORKER_MEMORY_BYTES).map_err(|_| Error::Invalid)?;
        let existing = reserve(&budget, MemoryKind::DisplayFrame, 1)?;
        assert_eq!(
            reserve(&budget, MemoryKind::CodecWorkspace, WORKER_MEMORY_BYTES).err(),
            Some(Error::Memory {
                estimated: WORKER_MEMORY_BYTES + 1,
                budget: WORKER_MEMORY_BYTES
            })
        );
        assert_eq!(budget.used_bytes(), 1);
        drop(existing);
        let worker = reserve(&budget, MemoryKind::CodecWorkspace, WORKER_MEMORY_BYTES)?;
        assert!(worker.belongs_to(&budget));
        assert_eq!(budget.used_bytes(), WORKER_MEMORY_BYTES);
        drop(worker);
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn returned_inventory_retains_its_charge_until_owner_retirement() -> Result<()> {
        let budget = SessionBudget::new(MANIFEST_OWNER_BYTES).map_err(|_| Error::Invalid)?;
        let charge = reserve(&budget, MemoryKind::Transfer, MANIFEST_OWNER_BYTES)?;
        let source = Source {
            original: [7; 32],
            original_bytes: 1,
            width: 1,
            height: 1,
            orientation: 1,
            bit_depth: 8,
            icc: None,
            display_conversion: false,
        };
        let pyramid = source.validate().map_err(|_| Error::Invalid)?;
        let records = pyramid
            .keys()
            .map(|key| TileRecord {
                key,
                encoded_bytes: 1,
                hash: [9; 32],
            })
            .collect();
        let manifest = Manifest::new(source, records).map_err(|_| Error::Invalid)?;
        let owner = BudgetedManifest { manifest, charge };
        assert_eq!(owner.manifest().tiles().len(), 1);
        assert!(owner.charge().belongs_to(&budget));
        assert_eq!(budget.used_bytes(), MANIFEST_OWNER_BYTES);
        let (manifest, charge) = owner.into_parts();
        assert_eq!(budget.used_bytes(), MANIFEST_OWNER_BYTES);
        drop(manifest);
        assert_eq!(budget.used_bytes(), MANIFEST_OWNER_BYTES);
        drop(charge);
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }

    #[test]
    fn failed_reservation_does_not_credit_other_owners() -> Result<()> {
        let budget = SessionBudget::new(4096).map_err(|_| Error::Invalid)?;
        let pinned = reserve(&budget, MemoryKind::Foreign, 3072)?;
        assert!(matches!(
            reserve(&budget, MemoryKind::Transfer, 2048),
            Err(Error::Memory {
                estimated: 5120,
                budget: 4096
            })
        ));
        assert_eq!(budget.used_bytes(), 3072);
        drop(pinned);
        assert_eq!(budget.used_bytes(), 0);
        Ok(())
    }
}

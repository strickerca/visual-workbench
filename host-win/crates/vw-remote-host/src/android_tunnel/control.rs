//! A one-shot, session/device/binary-bound owner-control grant.
use super::{
    Error, Result,
    device::{DeviceToken, SelectedDevice},
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionMode {
    ViewOnly,
    OwnerControl,
}

#[cfg(any(windows, test))]
pub(super) fn mirror_argv(serial: &str, mode: InteractionMode) -> Vec<String> {
    let mut argv = vec![format!("--serial={serial}"), "--no-audio".into()];
    if mode == InteractionMode::ViewOnly {
        argv.push("--no-control".into());
    }
    argv
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractionState {
    ViewOnly { mirror_revision: u64 },
    OwnerControl { mirror_revision: u64 },
    RetirementPending,
    Closed,
}

/// The target root must actually display and have the owner confirm. It does
/// not expose the private adb serial or a device command route.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerTargetDescriptor {
    pub(super) model: String,
    pub(super) transport_id: u32,
}

impl OwnerTargetDescriptor {
    pub(super) fn selected(device: &SelectedDevice) -> Self {
        Self {
            model: device.model.clone(),
            transport_id: device.transport_id,
        }
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn transport_id(&self) -> u32 {
        self.transport_id
    }
}

#[derive(Clone)]
pub struct ControlChallenge {
    pub(super) session: [u8; 16],
    pub(super) generation: u64,
    pub(super) device: DeviceToken,
    pub(super) binaries: [u8; 32],
    pub(super) nonce: [u8; 16],
    pub(super) mirror_revision: u64,
    pub(super) target: OwnerTargetDescriptor,
}

impl ControlChallenge {
    pub fn target(&self) -> &OwnerTargetDescriptor {
        &self.target
    }
}

pub struct OwnerControlPermit {
    binding: ControlChallenge,
    expires_at: Instant,
    consumed: bool,
}

impl OwnerControlPermit {
    /// Root-only mint boundary. Root must call this only after displaying the
    /// challenge target and receiving a fresh explicit local owner confirmation.
    /// No public bool, receipt, source hash or peer message can invoke it.
    pub(crate) fn mint_from_root_confirmation(
        challenge: &ControlChallenge,
        confirmed_target: &OwnerTargetDescriptor,
        lifetime: Duration,
    ) -> Result<Self> {
        if confirmed_target != &challenge.target
            || lifetime.is_zero()
            || lifetime > Duration::from_secs(30)
        {
            return Err(Error::Consent);
        }
        let expires_at = Instant::now().checked_add(lifetime).ok_or(Error::Limit)?;
        Ok(Self {
            binding: challenge.clone(),
            expires_at,
            consumed: false,
        })
    }

    pub(super) fn consume(&mut self, actual: &ControlChallenge, now: Instant) -> Result<()> {
        if std::mem::replace(&mut self.consumed, true) {
            return Err(Error::Consent);
        }
        let expected = &self.binding;
        if now >= self.expires_at
            || expected.session != actual.session
            || expected.generation != actual.generation
            || expected.device != actual.device
            || expected.binaries != actual.binaries
            || expected.nonce != actual.nonce
            || expected.mirror_revision != actual.mirror_revision
            || expected.target != actual.target
        {
            return Err(Error::Consent);
        }
        Ok(())
    }

    pub(super) fn reject(&mut self) {
        self.consumed = true;
    }

    #[cfg(test)]
    pub(super) fn synthetic_for_test(
        challenge: &ControlChallenge,
        lifetime: Duration,
    ) -> Result<Self> {
        Self::mint_from_root_confirmation(challenge, &challenge.target, lifetime)
    }
}

#[cfg(any(windows, test))]
pub(super) fn fresh_nonce() -> Result<[u8; 16]> {
    use ring::rand::SecureRandom;
    let mut nonce = [0u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error::Platform)?;
    Ok(nonce)
}

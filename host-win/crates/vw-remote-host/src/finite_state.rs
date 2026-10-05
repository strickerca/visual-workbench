//! Bounded actual-prefix ownership. Never infer global UP from process exit.
use crate::{Error, Result};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Button {
    Left,
    Right,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    Button(Button),
    Key { code: u16, extended: bool },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    Neutral,
    Down(Held),
    Up(Held),
}
#[derive(Default)]
pub(crate) struct Ledger {
    held: Vec<Held>,
    unknown: bool,
}
impl Ledger {
    pub fn plan(events: &[Event]) -> Result<()> {
        if events.is_empty() || events.len() > 16 {
            return Err(Error::Limit);
        }
        let mut planned = Self::default();
        for event in events {
            planned.step(*event)?;
        }
        if !planned.held.is_empty() {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    fn step(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Neutral => Ok(()),
            Event::Down(value) => {
                if self.held.contains(&value) {
                    return Err(Error::Invalid);
                }
                self.held.push(value);
                Ok(())
            }
            Event::Up(value) => {
                let index = self
                    .held
                    .iter()
                    .position(|v| *v == value)
                    .ok_or(Error::Invalid)?;
                self.held.remove(index);
                Ok(())
            }
        }
    }
    pub fn accepted(&mut self, events: &[Event], count: u32) -> Result<()> {
        if self.pending() {
            return Err(Error::RetirementPending);
        }
        let count = count as usize;
        if count > events.len() {
            self.unknown = true;
            return Err(Error::Invalid);
        }
        for event in &events[..count] {
            if let Err(error) = self.step(*event) {
                self.unknown = true;
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn pending(&self) -> bool {
        self.unknown || !self.held.is_empty()
    }
    pub fn count(&self) -> u32 {
        self.held.len() as u32
    }
    pub fn unknown(&self) -> bool {
        self.unknown
    }
    pub fn last(&self) -> Option<Held> {
        self.held.last().copied()
    }
    pub fn release_one(
        &mut self,
        value: Held,
        guard: impl FnOnce() -> Result<()>,
        send: impl FnOnce() -> u32,
    ) -> Result<()> {
        if self.last() != Some(value) || self.unknown {
            return Err(Error::RetirementPending);
        }
        guard()?;
        self.released(value, send())
    }
    pub fn released(&mut self, value: Held, count: u32) -> Result<()> {
        if count == 0 {
            return Err(Error::PartialInput);
        }
        if count != 1 || self.held.last() != Some(&value) {
            self.unknown = true;
            return Err(Error::Invalid);
        }
        self.held.pop();
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn key() -> Held {
        Held::Key {
            code: 90,
            extended: false,
        }
    }
    fn click() -> [Event; 3] {
        [
            Event::Neutral,
            Event::Down(Held::Button(Button::Left)),
            Event::Up(Held::Button(Button::Left)),
        ]
    }
    #[test]
    fn full_balanced_batch_has_no_retained_down() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 3)?;
        assert!(!l.pending());
        Ok(())
    }
    #[test]
    fn mouse_prefix_owns_only_actual_down() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 2)?;
        assert_eq!(l.count(), 1);
        assert_eq!(l.last(), Some(Held::Button(Button::Left)));
        Ok(())
    }
    #[test]
    fn movement_only_partial_never_invents_button() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 1)?;
        assert!(!l.pending());
        Ok(())
    }
    #[test]
    fn zero_accepted_never_owns_down() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 0)?;
        assert!(!l.pending());
        Ok(())
    }
    #[test]
    fn keyboard_prefix_preserves_extended_key_identity() -> Result<()> {
        let mut l = Ledger::default();
        let k = Held::Key {
            code: 39,
            extended: true,
        };
        l.accepted(&[Event::Down(k), Event::Up(k)], 1)?;
        assert_eq!(l.last(), Some(k));
        Ok(())
    }
    #[test]
    fn failed_release_keeps_same_live_ledger() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&[Event::Down(key()), Event::Up(key())], 1)?;
        assert_eq!(l.released(key(), 0), Err(Error::PartialInput));
        assert!(l.pending());
        assert_eq!(l.count(), 1);
        Ok(())
    }
    #[test]
    fn accepted_release_clears_only_exact_owned_down() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&[Event::Down(key()), Event::Up(key())], 1)?;
        l.released(key(), 1)?;
        assert!(!l.pending());
        Ok(())
    }
    #[test]
    fn unknown_count_cannot_be_cleared_by_empty_ledger() {
        let mut l = Ledger::default();
        assert_eq!(l.accepted(&click(), 4), Err(Error::Invalid));
        assert!(l.pending());
        assert!(l.unknown());
        assert_eq!(l.count(), 0)
    }
    #[test]
    fn duplicate_down_and_foreign_up_refused_before_dispatch() {
        assert_eq!(
            Ledger::plan(&[Event::Down(key()), Event::Down(key()), Event::Up(key())]),
            Err(Error::Invalid)
        );
        assert_eq!(Ledger::plan(&[Event::Up(key())]), Err(Error::Invalid))
    }
    #[test]
    fn unbalanced_and_over_limit_refused_before_dispatch() {
        assert_eq!(Ledger::plan(&[Event::Down(key())]), Err(Error::Invalid));
        assert_eq!(Ledger::plan(&[Event::Neutral; 17]), Err(Error::Limit))
    }
    #[test]
    fn overlapping_new_batch_cannot_replace_pending_owner() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 2)?;
        assert_eq!(l.accepted(&click(), 3), Err(Error::RetirementPending));
        assert_eq!(l.count(), 1);
        Ok(())
    }
    #[test]
    fn modifier_prefix_retires_exact_reverse_owned_order() -> Result<()> {
        let c = Held::Key {
            code: 17,
            extended: false,
        };
        let z = key();
        let mut l = Ledger::default();
        l.accepted(
            &[Event::Down(c), Event::Down(z), Event::Up(z), Event::Up(c)],
            2,
        )?;
        assert_eq!(l.last(), Some(z));
        l.released(z, 1)?;
        assert_eq!(l.last(), Some(c));
        l.released(c, 1)?;
        assert!(!l.pending());
        Ok(())
    }
    #[test]
    fn failed_release_guard_never_calls_native_producer() -> Result<()> {
        let mut l = Ledger::default();
        l.accepted(&click(), 2)?;
        let called = std::cell::Cell::new(false);
        assert_eq!(
            l.release_one(
                Held::Button(Button::Left),
                || Err(Error::TargetChanged),
                || {
                    called.set(true);
                    1
                }
            ),
            Err(Error::TargetChanged)
        );
        assert!(!called.get());
        assert!(l.pending());
        Ok(())
    }
    #[test]
    fn release_rechecks_guard_for_every_known_down() -> Result<()> {
        let c = Held::Key {
            code: 17,
            extended: false,
        };
        let z = key();
        let mut l = Ledger::default();
        l.accepted(
            &[Event::Down(c), Event::Down(z), Event::Up(z), Event::Up(c)],
            2,
        )?;
        l.release_one(z, || Ok(()), || 1)?;
        assert_eq!(
            l.release_one(c, || Err(Error::TargetChanged), || 1),
            Err(Error::TargetChanged)
        );
        assert_eq!(l.last(), Some(c));
        Ok(())
    }
}

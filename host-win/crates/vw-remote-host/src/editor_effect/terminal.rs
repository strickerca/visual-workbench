//! Source-owned release rendezvous. Neither EOF nor process death is a release.
use crate::{Error, Result, platform::finite_input::FiniteInputOwner};
use serde::Serialize;
use std::io::Write;
use vw_remote::wire::FiniteRetirement;

#[derive(Default)]
pub(super) struct Producer {
    attempted: bool,
    terminal: bool,
}
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
struct Record<'a> {
    schema: u32,
    operation: &'static str,
    retirement: &'a FiniteRetirement,
}
fn record_to(mut output: impl Write, retirement: &FiniteRetirement) -> Result<()> {
    let bytes = serde_json::to_vec(&Record {
        schema: 1,
        operation: "finite-retirement",
        retirement,
    })
    .map_err(|_| Error::Invalid)?;
    if bytes.len() > 4096 {
        return Err(Error::Limit);
    }
    output.write_all(&bytes)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
trait RetirementPort {
    fn retire(&mut self) -> FiniteRetirement;
    fn wait_retired(&mut self);
}
impl<S: 'static> RetirementPort for FiniteInputOwner<S> {
    fn retire(&mut self) -> FiniteRetirement {
        FiniteInputOwner::retire(self)
    }
    fn wait_retired(&mut self) {
        FiniteInputOwner::wait_retired(self);
    }
}
fn retire_to(owner: &mut impl RetirementPort, output: &mut impl Write) -> Result<()> {
    let retirement = owner.retire();
    let pending_output = if matches!(retirement, FiniteRetirement::Complete) {
        Ok(())
    } else {
        record_to(&mut *output, &retirement)
    };
    // A failed Pending flush must never bypass actual release-only retirement.
    owner.wait_retired();
    if !matches!(owner.retire(), FiniteRetirement::Complete) {
        return Err(Error::RetirementPending);
    }
    let complete_output = record_to(output, &FiniteRetirement::Complete);
    pending_output.and(complete_output)
}
impl Producer {
    pub(super) fn arm_attempt(&mut self) -> Result<()> {
        if self.terminal {
            return Err(Error::RetirementPending);
        }
        self.attempted = true;
        Ok(())
    }
    pub(super) fn finish<S: 'static>(&mut self, owner: &mut FiniteInputOwner<S>) -> Result<()> {
        if self.terminal {
            return Err(Error::Invalid);
        }
        // Always retain and perform actual release even if writing Pending fails.
        // A failed/partial pipe write never authorizes the parent to infer release.
        let result = retire_to(owner, &mut std::io::stdout().lock());
        // Sealed before returning even on IO failure: no subsequent input is legal.
        self.terminal = true;
        result
    }
    pub(super) fn complete_without_input(&mut self) -> Result<()> {
        if self.attempted || self.terminal {
            return Err(Error::Invalid);
        }
        self.terminal = true;
        record_to(std::io::stdout().lock(), &FiniteRetirement::Complete)
    }
    pub(super) fn failed_before_input(&mut self) -> Result<()> {
        if !self.attempted && !self.terminal {
            self.complete_without_input()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_release_record_is_bounded_exact_and_newline_terminated() -> Result<()> {
        let mut output = vec![];
        record_to(&mut output, &FiniteRetirement::Complete)?;
        assert_eq!(output.last(), Some(&b'\n'));
        assert!(output.len() <= 4097);
        let value: serde_json::Value =
            serde_json::from_slice(&output).map_err(|_| Error::Invalid)?;
        assert_eq!(
            value,
            serde_json::json!({"schema":1,"operation":"finite-retirement","retirement":"Complete"})
        );
        Ok(())
    }
    #[test]
    fn an_attempt_or_terminal_prevents_a_no_input_release_claim() {
        let mut producer = Producer {
            attempted: true,
            terminal: false,
        };
        assert_eq!(producer.complete_without_input(), Err(Error::Invalid));
        producer.terminal = true;
        assert_eq!(producer.arm_attempt(), Err(Error::RetirementPending));
    }
    struct FixtureOwner {
        retired: bool,
        wait_called: bool,
    }
    impl RetirementPort for FixtureOwner {
        fn retire(&mut self) -> FiniteRetirement {
            if self.retired {
                FiniteRetirement::Complete
            } else {
                FiniteRetirement::Pending {
                    held_count: 1,
                    uncertain: false,
                    error: Error::TargetChanged,
                }
            }
        }
        fn wait_retired(&mut self) {
            self.wait_called = true;
            self.retired = true;
        }
    }
    #[test]
    fn pending_flush_precedes_wait_and_terminal_follows_actual_zero_held() -> Result<()> {
        use std::{cell::RefCell, rc::Rc};
        struct Owner {
            retired: bool,
            trace: Rc<RefCell<Vec<&'static str>>>,
        }
        impl RetirementPort for Owner {
            fn retire(&mut self) -> FiniteRetirement {
                if self.retired {
                    FiniteRetirement::Complete
                } else {
                    FiniteRetirement::Pending {
                        held_count: 1,
                        uncertain: false,
                        error: Error::TargetChanged,
                    }
                }
            }
            fn wait_retired(&mut self) {
                self.trace.borrow_mut().push("wait");
                self.retired = true;
            }
        }
        struct Output {
            bytes: Vec<u8>,
            flushed: usize,
            trace: Rc<RefCell<Vec<&'static str>>>,
        }
        impl Write for Output {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                let value: serde_json::Value = serde_json::from_slice(&self.bytes[self.flushed..])
                    .map_err(std::io::Error::other)?;
                self.trace
                    .borrow_mut()
                    .push(if value["retirement"] == "Complete" {
                        "complete_flush"
                    } else {
                        "pending_flush"
                    });
                self.flushed = self.bytes.len();
                Ok(())
            }
        }
        let trace = Rc::new(RefCell::new(vec![]));
        let mut owner = Owner {
            retired: false,
            trace: trace.clone(),
        };
        let mut output = Output {
            bytes: vec![],
            flushed: 0,
            trace: trace.clone(),
        };
        retire_to(&mut owner, &mut output)?;
        assert_eq!(*trace.borrow(), ["pending_flush", "wait", "complete_flush"]);
        let bytes = output.bytes;
        let lines = bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 2);
        let first: serde_json::Value =
            serde_json::from_slice(lines[0]).map_err(|_| Error::Invalid)?;
        let last: serde_json::Value =
            serde_json::from_slice(lines[1]).map_err(|_| Error::Invalid)?;
        assert!(first["retirement"]["Pending"].is_object());
        assert_eq!(last["retirement"], "Complete");
        Ok(())
    }
    #[test]
    fn broken_pending_pipe_still_waits_and_retires_actual_owner() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut owner = FixtureOwner {
            retired: false,
            wait_called: false,
        };
        assert!(retire_to(&mut owner, &mut Broken).is_err());
        assert!(owner.wait_called && owner.retired);
    }
}

//! Bounded effect-only terminal parser. Completion is source-owned UP proof.
use crate::{Error, Result};
use serde::Deserialize;
use std::{
    io::{BufReader, Read},
    sync::atomic::{AtomicBool, Ordering},
};
use vw_remote::wire::FiniteRetirement;
const MAX_RECORD: usize = 4096;
const MAX_OUTPUT: usize = super::MAX_PROBE_OUTPUT;
const MAX_TOTAL: usize = MAX_OUTPUT + 2 * (MAX_RECORD + 1);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    operation: String,
    retirement: FiniteRetirement,
}
#[derive(Default)]
struct Parser {
    pending: bool,
    complete: bool,
    final_receipt: Option<Vec<u8>>,
}
impl Parser {
    fn line(&mut self, bytes: Vec<u8>, uncertain: &AtomicBool) -> Result<()> {
        if bytes.is_empty() || self.final_receipt.is_some() {
            return Err(Error::Invalid);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| Error::Invalid)?;
        if value.get("operation").and_then(serde_json::Value::as_str) == Some("finite-retirement") {
            if bytes.len() > MAX_RECORD || self.complete {
                return Err(Error::Invalid);
            }
            let record: Record = serde_json::from_value(value).map_err(|_| Error::Invalid)?;
            if record.schema != 1 || record.operation != "finite-retirement" {
                return Err(Error::Invalid);
            }
            match record.retirement {
                FiniteRetirement::Pending {
                    held_count,
                    uncertain: unknown,
                    ..
                } => {
                    if self.pending || held_count > 16 || (held_count == 0 && !unknown) {
                        return Err(Error::Invalid);
                    }
                    self.pending = true;
                }
                FiniteRetirement::Complete => {
                    self.complete = true;
                    // Only this closed record from the pinned suspended child
                    // clears input uncertainty. Later malformed final output is
                    // still a failed invocation, but cannot revoke actual UP.
                    uncertain.store(false, Ordering::Release);
                }
            }
        } else {
            if !self.complete || bytes.len() > MAX_OUTPUT {
                return Err(Error::Invalid);
            }
            super::measured_effect_output(&bytes)?;
            self.final_receipt = Some(bytes);
        }
        Ok(())
    }
    fn finish(self) -> Result<Vec<u8>> {
        if !self.complete {
            return Err(Error::RetirementPending);
        }
        self.final_receipt.ok_or(Error::Invalid)
    }
}
pub(super) fn read(input: &mut impl Read, uncertain: &AtomicBool) -> Result<Vec<u8>> {
    let mut input = BufReader::with_capacity(8192, input);
    let mut parser = Parser::default();
    let mut line = Vec::new();
    let mut total = 0usize;
    let mut byte = [0u8; 1];
    while input.read(&mut byte)? != 0 {
        total = total.checked_add(1).ok_or(Error::Limit)?;
        if total > MAX_TOTAL {
            return Err(Error::Limit);
        }
        if byte[0] == b'\n' {
            parser.line(std::mem::take(&mut line), uncertain)?;
        } else {
            if line.len() >= MAX_OUTPUT {
                return Err(Error::Limit);
            }
            line.push(byte[0]);
        }
    }
    // A partial line/EOF is never a release record; process death is not UP.
    if !line.is_empty() {
        return Err(Error::Invalid);
    }
    parser.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn complete() -> Vec<u8> {
        b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":\"Complete\"}\n".to_vec()
    }
    fn pending() -> Vec<u8> {
        b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":{\"Pending\":{\"held_count\":1,\"uncertain\":false,\"error\":\"TargetChanged\"}}}\n".to_vec()
    }
    fn final_receipt() -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec(&serde_json::json!({"schema":1,"operation":"owned-blank-editor-effect-hil",
            "profile_authority":false,"parent_job_present":true,"held_process_and_image_namespace":true,
            "owned_blank_fixture_asserted":true,"input_sent":false,"target":{},"image":{},"before":{}})).map_err(|_|Error::Invalid)?;
        bytes.push(b'\n');
        Ok(bytes)
    }
    #[test]
    fn pending_then_complete_then_exact_final_clears_only_at_terminal() -> Result<()> {
        let uncertain = AtomicBool::new(true);
        let mut parser = Parser::default();
        let mut first = pending();
        first.pop();
        parser.line(first, &uncertain)?;
        assert!(uncertain.load(Ordering::Acquire));
        let mut second = complete();
        second.pop();
        parser.line(second, &uncertain)?;
        assert!(!uncertain.load(Ordering::Acquire));
        let mut last = final_receipt()?;
        last.pop();
        parser.line(last, &uncertain)?;
        assert!(parser.finish().is_ok());
        Ok(())
    }
    #[test]
    fn eof_pending_partial_terminal_and_death_cannot_clear_input_owner() {
        for bytes in [
            vec![],
            pending(),
            complete()[..complete().len() - 1].to_vec(),
            b"child died\n".to_vec(),
        ] {
            let uncertain = AtomicBool::new(true);
            assert!(read(&mut Cursor::new(bytes), &uncertain).is_err());
            assert!(uncertain.load(Ordering::Acquire));
        }
    }
    #[test]
    fn final_before_complete_or_unknown_terminal_fields_refuse() -> Result<()> {
        for bytes in [final_receipt()?, b"{\"schema\":1,\"operation\":\"finite-retirement\",\"retirement\":\"Complete\",\"effect\":true}\n".to_vec()] {
            let uncertain = AtomicBool::new(true);
            assert!(read(&mut Cursor::new(bytes), &uncertain).is_err());
            assert!(uncertain.load(Ordering::Acquire));
        }
        Ok(())
    }
    #[test]
    fn malformed_final_after_complete_is_failed_but_keeps_actual_up_proof() {
        let uncertain = AtomicBool::new(true);
        let mut bytes = complete();
        bytes.extend_from_slice(b"invalid final\n");
        assert!(read(&mut Cursor::new(bytes), &uncertain).is_err());
        assert!(!uncertain.load(Ordering::Acquire));
    }
    #[test]
    fn duplicate_pending_and_excess_bytes_preserve_uncertain_owner() {
        let mut twice = pending();
        twice.extend(pending());
        for bytes in [twice, vec![b'x'; MAX_OUTPUT + 1]] {
            let uncertain = AtomicBool::new(true);
            assert!(read(&mut Cursor::new(bytes), &uncertain).is_err());
            assert!(uncertain.load(Ordering::Acquire));
        }
    }
    #[test]
    fn complete_without_pending_is_valid_but_duplicate_or_extra_final_fails() -> Result<()> {
        let mut bytes = complete();
        bytes.extend(final_receipt()?);
        let uncertain = AtomicBool::new(true);
        read(&mut Cursor::new(bytes.clone()), &uncertain)?;
        assert!(!uncertain.load(Ordering::Acquire));
        bytes.extend(final_receipt()?);
        assert!(read(&mut Cursor::new(bytes), &uncertain).is_err());
        let mut twice = complete();
        twice.extend(complete());
        assert!(read(&mut Cursor::new(twice), &AtomicBool::new(true)).is_err());
        Ok(())
    }
}

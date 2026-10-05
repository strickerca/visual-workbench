//! Only the owned native helper's successful guarded injection receipts enter
//! this ledger. Transport admission and keepalive do not call record().
use crate::{Binding, Error, Result, Scope};
use std::collections::VecDeque;
const LIMIT: usize = 4096;
#[derive(Clone, Debug)]
pub struct Receipt {
    pub binding: Binding,
    pub input_seq: u64,
    pub accepted_qpc_100ns: u64,
}
pub struct Applied {
    binding: Binding,
    receipts: VecDeque<Receipt>,
    last_seq: u64,
    last_qpc: u64,
    retired: bool,
}
impl Applied {
    pub fn new(binding: Binding) -> Result<Self> {
        binding.validate()?;
        Ok(Self {
            binding,
            receipts: VecDeque::new(),
            last_seq: 0,
            last_qpc: 0,
            retired: false,
        })
    }
    pub fn record(&mut self, receipt: Receipt) -> Result<()> {
        if self.retired
            || receipt.binding != self.binding
            || receipt.input_seq <= self.last_seq
            || receipt.accepted_qpc_100ns == 0
            || receipt.accepted_qpc_100ns < self.last_qpc
        {
            self.retire();
            return Err(Error::Invalid);
        }
        self.last_seq = receipt.input_seq;
        self.last_qpc = receipt.accepted_qpc_100ns;
        if self.receipts.len() == LIMIT {
            self.receipts.pop_front();
        }
        self.receipts.push_back(receipt);
        Ok(())
    }
    pub fn for_frame(&self, scope: &Scope, capture_qpc_100ns: u64) -> (Option<String>, u64) {
        if self.retired || &self.binding.scope != scope || capture_qpc_100ns == 0 {
            return (None, 0);
        }
        let seq = self
            .receipts
            .iter()
            .rev()
            .find(|r| r.accepted_qpc_100ns <= capture_qpc_100ns)
            .map_or(0, |r| r.input_seq);
        (Some(self.binding.input_session_id.clone()), seq)
    }
    pub fn retire(&mut self) {
        self.retired = true;
        self.receipts.clear();
    }
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
}

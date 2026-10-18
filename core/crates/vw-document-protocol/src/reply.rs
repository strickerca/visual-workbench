use crate::{
    JobBinding, MAX_DATA, MAX_OUTPUT, MAX_REPLY_METADATA, MAX_WIRE_OVERHEAD, ProtocolError,
    ProtocolResult, buffer, json, read_exact, write_all,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const MAGIC: &[u8; 4] = b"VWR1";
const HEADER: usize = 28;
const MAX_PROGRESS: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FrameKind {
    Metadata = 1,
    Data = 2,
    Progress = 3,
    Complete = 4,
    Fault = 5,
}
impl FrameKind {
    fn parse(value: u8) -> ProtocolResult<Self> {
        match value {
            1 => Ok(Self::Metadata),
            2 => Ok(Self::Data),
            3 => Ok(Self::Progress),
            4 => Ok(Self::Complete),
            5 => Ok(Self::Fault),
            _ => Err(ProtocolError::Invalid),
        }
    }
    fn limit(self) -> usize {
        match self {
            Self::Metadata => MAX_REPLY_METADATA,
            Self::Data => MAX_DATA,
            Self::Progress | Self::Complete | Self::Fault => 4096,
        }
    }
}

pub struct Frame {
    pub kind: FrameKind,
    pub payload: Vec<u8>,
}
impl std::fmt::Debug for Frame {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        output
            .debug_struct("Frame")
            .field("kind", &self.kind)
            .field("bytes", &self.payload.len())
            .finish()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progress {
    pub completed: u64,
    pub total: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Completion {
    pub binding: JobBinding,
    pub bytes: u64,
    pub output_blake3: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaultCode {
    Invalid,
    Limit,
    Memory,
    Cancelled,
    Password,
    Unsupported,
    Malformed,
    Native,
    Output,
    Verification,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitMetadata,
    Streaming,
    Complete,
    Fault,
    Failed,
}

struct WireBudget {
    data: u64,
    overhead: u64,
    output_limit: u64,
}
impl WireBudget {
    fn new(output_limit: u64) -> ProtocolResult<Self> {
        if output_limit == 0 || output_limit > MAX_OUTPUT {
            return Err(ProtocolError::Limit);
        }
        Ok(Self {
            data: 0,
            overhead: 0,
            output_limit,
        })
    }
    fn admit(&mut self, kind: FrameKind, payload_length: usize) -> ProtocolResult<()> {
        let payload = payload_length as u64;
        let data = self
            .data
            .checked_add(if kind == FrameKind::Data { payload } else { 0 })
            .ok_or(ProtocolError::Limit)?;
        let overhead = self
            .overhead
            .checked_add(HEADER as u64)
            .and_then(|n| n.checked_add(if kind == FrameKind::Data { 0 } else { payload }))
            .ok_or(ProtocolError::Limit)?;
        if data > self.output_limit || overhead > MAX_WIRE_OVERHEAD {
            return Err(ProtocolError::Limit);
        }
        self.data = data;
        self.overhead = overhead;
        Ok(())
    }
}

pub struct ReplyWriter<'a> {
    writer: &'a mut dyn Write,
    binding: JobBinding,
    limit: u64,
    count: u64,
    hash: blake3::Hasher,
    state: State,
    progress_count: u32,
    previous_progress: Option<Progress>,
    wire: WireBudget,
}
impl<'a> ReplyWriter<'a> {
    pub fn new(
        writer: &'a mut dyn Write,
        binding: JobBinding,
        output_limit: u64,
    ) -> ProtocolResult<Self> {
        if binding.nonce == [0; 16] || output_limit == 0 || output_limit > MAX_OUTPUT {
            return Err(ProtocolError::Limit);
        }
        Ok(Self {
            writer,
            binding,
            limit: output_limit,
            count: 0,
            hash: blake3::Hasher::new(),
            state: State::AwaitMetadata,
            progress_count: 0,
            previous_progress: None,
            wire: WireBudget::new(output_limit)?,
        })
    }
    pub fn metadata<T: Serialize>(&mut self, metadata: &T) -> ProtocolResult<()> {
        if self.state != State::AwaitMetadata {
            return Err(ProtocolError::Order);
        }
        let payload = match json(metadata, MAX_REPLY_METADATA) {
            Ok(value) => value,
            Err(error) => {
                self.state = State::Failed;
                return Err(error);
            }
        };
        self.frame(FrameKind::Metadata, &payload)?;
        self.state = State::Streaming;
        Ok(())
    }
    pub fn data(&mut self, bytes: &[u8]) -> ProtocolResult<()> {
        if self.state != State::Streaming {
            return Err(ProtocolError::Order);
        }
        let Some(next) = self.count.checked_add(bytes.len() as u64) else {
            self.state = State::Failed;
            return Err(ProtocolError::Limit);
        };
        if next > self.limit {
            self.state = State::Failed;
            return Err(ProtocolError::Limit);
        }
        for block in bytes.chunks(MAX_DATA) {
            self.frame(FrameKind::Data, block)?;
        }
        self.count = next;
        self.hash.update(bytes);
        Ok(())
    }
    /// Adapter throttles progress to at most 4096 numeric updates per job.
    pub fn progress(&mut self, progress: Progress) -> ProtocolResult<()> {
        if self.state != State::Streaming {
            return Err(ProtocolError::Order);
        }
        if let Err(error) = check_progress(self.previous_progress, self.progress_count, progress) {
            self.state = State::Failed;
            return Err(error);
        }
        self.frame(FrameKind::Progress, &json(&progress, 4096)?)?;
        self.previous_progress = Some(progress);
        self.progress_count += 1;
        Ok(())
    }
    pub fn finish(mut self) -> ProtocolResult<Completion> {
        if self.state != State::Streaming {
            return Err(ProtocolError::Order);
        }
        let result = Completion {
            binding: self.binding,
            bytes: self.count,
            output_blake3: *self.hash.finalize().as_bytes(),
        };
        self.frame(FrameKind::Complete, &json(&result, 4096)?)?;
        self.writer
            .flush()
            .map_err(|error| ProtocolError::Io(error.kind()))?;
        self.state = State::Complete;
        Ok(result)
    }
    pub fn fault(mut self, code: FaultCode) -> ProtocolResult<()> {
        if matches!(self.state, State::Complete | State::Fault) {
            return Err(ProtocolError::Order);
        }
        self.frame(FrameKind::Fault, &json(&code, 4096)?)?;
        self.writer
            .flush()
            .map_err(|error| ProtocolError::Io(error.kind()))?;
        self.state = State::Fault;
        Ok(())
    }
    fn frame(&mut self, kind: FrameKind, payload: &[u8]) -> ProtocolResult<()> {
        if payload.is_empty() || payload.len() > kind.limit() {
            return Err(ProtocolError::Limit);
        }
        if let Err(error) = self.wire.admit(kind, payload.len()) {
            self.state = State::Failed;
            return Err(error);
        }
        let mut header = [0u8; HEADER];
        header[..4].copy_from_slice(MAGIC);
        header[4] = kind as u8;
        header[8..12].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        header[12..28].copy_from_slice(&self.binding.nonce);
        // Any error leaves the stream incomplete. A parent must reject it; a
        // later operation may not turn partial data into a valid Complete.
        if let Err(error) =
            write_all(self.writer, &header).and_then(|()| write_all(self.writer, payload))
        {
            self.state = State::Fault;
            return Err(error);
        }
        Ok(())
    }
}

pub struct ReplyReader<'a> {
    reader: &'a mut dyn Read,
    nonce: [u8; 16],
    failed: bool,
    wire: WireBudget,
}
impl<'a> ReplyReader<'a> {
    pub fn new(reader: &'a mut dyn Read, nonce: [u8; 16]) -> ProtocolResult<Self> {
        Self::with_output_limit(reader, nonce, MAX_OUTPUT)
    }
    pub fn with_output_limit(
        reader: &'a mut dyn Read,
        nonce: [u8; 16],
        output_limit: u64,
    ) -> ProtocolResult<Self> {
        if nonce == [0; 16] {
            return Err(ProtocolError::Invalid);
        }
        Ok(Self {
            reader,
            nonce,
            failed: false,
            wire: WireBudget::new(output_limit)?,
        })
    }
    pub fn next_frame(&mut self) -> ProtocolResult<Option<Frame>> {
        if self.failed {
            return Err(ProtocolError::Order);
        }
        let result = self.next_inner();
        if result.is_err() {
            self.failed = true;
        }
        result
    }
    fn next_inner(&mut self) -> ProtocolResult<Option<Frame>> {
        let mut header = [0u8; HEADER];
        loop {
            match self.reader.read(&mut header[..1]) {
                Ok(0) => return Ok(None),
                Ok(_) => break,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(ProtocolError::Io(error.kind())),
            }
        }
        read_exact(self.reader, &mut header[1..])?;
        if &header[..4] != MAGIC || header[5..8] != [0; 3] || header[12..28] != self.nonce {
            return Err(ProtocolError::Integrity);
        }
        let kind = FrameKind::parse(header[4])?;
        let length = u32::from_le_bytes(
            header[8..12]
                .try_into()
                .map_err(|_| ProtocolError::Invalid)?,
        ) as usize;
        if length == 0 || length > kind.limit() {
            return Err(ProtocolError::Limit);
        }
        // Charge the entire frame before allocating or reading its payload.
        // Tiny DATA fragments and padded JSON cannot hide in the DATA budget.
        self.wire.admit(kind, length)?;
        let mut payload = buffer(length)?;
        read_exact(self.reader, &mut payload)?;
        Ok(Some(Frame { kind, payload }))
    }
}

/// The caller may write Data only into owned staging while tracking. Publish
/// only after `finish_eof` and successful child exit/cleanup, plus app revision
/// checks and operation-specific metadata validation. This is framing integrity,
/// not a redaction/proof or decoder fidelity acceptance result.
pub struct ReplyTracker {
    binding: JobBinding,
    state: State,
    limit: u64,
    count: u64,
    hash: blake3::Hasher,
    completion: Option<Completion>,
    fault: Option<FaultCode>,
    progress_count: u32,
    previous_progress: Option<Progress>,
    wire: WireBudget,
}
impl ReplyTracker {
    pub fn new(binding: JobBinding, output_limit: u64) -> ProtocolResult<Self> {
        if binding.nonce == [0; 16] || output_limit == 0 || output_limit > MAX_OUTPUT {
            return Err(ProtocolError::Limit);
        }
        Ok(Self {
            binding,
            state: State::AwaitMetadata,
            limit: output_limit,
            count: 0,
            hash: blake3::Hasher::new(),
            completion: None,
            fault: None,
            progress_count: 0,
            previous_progress: None,
            wire: WireBudget::new(output_limit)?,
        })
    }
    pub fn accept(&mut self, frame: &Frame) -> ProtocolResult<()> {
        let result = self.accept_inner(frame);
        if result.is_err() {
            self.state = State::Failed;
        }
        result
    }
    fn accept_inner(&mut self, frame: &Frame) -> ProtocolResult<()> {
        if matches!(self.state, State::Complete | State::Fault | State::Failed) {
            return Err(ProtocolError::Order);
        }
        if frame.payload.is_empty() || frame.payload.len() > frame.kind.limit() {
            return Err(ProtocolError::Limit);
        }
        self.wire.admit(frame.kind, frame.payload.len())?;
        match frame.kind {
            FrameKind::Metadata => {
                if self.state != State::AwaitMetadata {
                    return Err(ProtocolError::Order);
                }
                // Caller validates the typed metadata against its captured
                // requested operation/dimensions before using any Data bytes.
                self.state = State::Streaming;
            }
            FrameKind::Data => {
                if self.state != State::Streaming {
                    return Err(ProtocolError::Order);
                }
                let next = self
                    .count
                    .checked_add(frame.payload.len() as u64)
                    .ok_or(ProtocolError::Limit)?;
                if next > self.limit {
                    return Err(ProtocolError::Limit);
                }
                self.hash.update(&frame.payload);
                self.count = next;
            }
            FrameKind::Progress => {
                if self.state != State::Streaming {
                    return Err(ProtocolError::Order);
                }
                let value: Progress =
                    serde_json::from_slice(&frame.payload).map_err(|_| ProtocolError::Invalid)?;
                check_progress(self.previous_progress, self.progress_count, value)?;
                self.previous_progress = Some(value);
                self.progress_count += 1;
            }
            FrameKind::Complete => {
                if self.state != State::Streaming {
                    return Err(ProtocolError::Order);
                }
                let value: Completion =
                    serde_json::from_slice(&frame.payload).map_err(|_| ProtocolError::Invalid)?;
                if value.binding != self.binding
                    || value.bytes != self.count
                    || value.output_blake3 != *self.hash.finalize().as_bytes()
                {
                    return Err(ProtocolError::Integrity);
                }
                self.completion = Some(value);
                self.state = State::Complete;
            }
            FrameKind::Fault => {
                self.fault = Some(
                    serde_json::from_slice(&frame.payload).map_err(|_| ProtocolError::Invalid)?,
                );
                self.state = State::Fault;
            }
        }
        Ok(())
    }
    pub fn fault_code(&self) -> Option<FaultCode> {
        self.fault
    }
    pub fn finish_eof(self) -> ProtocolResult<Completion> {
        if self.state != State::Complete {
            return Err(ProtocolError::Order);
        }
        self.completion.ok_or(ProtocolError::Order)
    }
}

fn check_progress(previous: Option<Progress>, count: u32, value: Progress) -> ProtocolResult<()> {
    if count >= MAX_PROGRESS
        || value.total == 0
        || value.completed > value.total
        || previous.is_some_and(|old| old.total != value.total || old.completed >= value.completed)
    {
        return Err(ProtocolError::Limit);
    }
    Ok(())
}

/// Strict EOF-complete consumption into a private writer. The enclosing process
/// owner still must require successful child exit and joined cleanup before
/// publication. `validate_metadata` checks the exact requested result schema.
pub fn receive_reply(
    reader: &mut dyn Read,
    private_output: &mut dyn Write,
    binding: JobBinding,
    output_limit: u64,
    validate_metadata: &mut dyn FnMut(&[u8]) -> ProtocolResult<()>,
    progress: &mut dyn FnMut(Progress),
) -> ProtocolResult<Completion> {
    let mut reader = ReplyReader::with_output_limit(reader, binding.nonce, output_limit)?;
    let mut tracker = ReplyTracker::new(binding, output_limit)?;
    while let Some(frame) = reader.next_frame()? {
        tracker.accept(&frame)?;
        match frame.kind {
            FrameKind::Metadata => validate_metadata(&frame.payload)?,
            FrameKind::Data => write_all(private_output, &frame.payload)?,
            FrameKind::Progress => progress(
                serde_json::from_slice(&frame.payload).map_err(|_| ProtocolError::Invalid)?,
            ),
            FrameKind::Complete => {}
            FrameKind::Fault => {
                return Err(ProtocolError::Worker(
                    tracker.fault_code().ok_or(ProtocolError::Invalid)?,
                ));
            }
        }
    }
    let complete = tracker.finish_eof()?;
    private_output
        .flush()
        .map_err(|error| ProtocolError::Io(error.kind()))?;
    Ok(complete)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // Test-only accounting fixture setup.
    use super::*;
    #[test]
    fn maximum_normal_blocks_and_numeric_progress_fit_without_large_allocations() {
        let mut budget = WireBudget::new(MAX_OUTPUT).unwrap();
        budget
            .admit(FrameKind::Metadata, MAX_REPLY_METADATA)
            .unwrap();
        for _ in 0..MAX_OUTPUT / MAX_DATA as u64 {
            budget.admit(FrameKind::Data, MAX_DATA).unwrap();
        }
        for _ in 0..MAX_PROGRESS {
            budget.admit(FrameKind::Progress, 80).unwrap();
        }
        budget.admit(FrameKind::Complete, 4096).unwrap();
        assert_eq!(budget.data, MAX_OUTPUT);
        assert!(budget.overhead < MAX_WIRE_OVERHEAD);
    }
}

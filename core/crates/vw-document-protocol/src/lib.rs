//! Private one-job worker protocol. No filenames, endpoints, commands or freeform
//! diagnostics cross the parser boundary. Replies are untrusted until completed.
#![forbid(unsafe_code)]

mod job;
mod reply;
pub use job::{
    CopyKind, Job, JobBinding, JobLimits, Operation, Overlay, PageRegion, Request, Tile, read_job,
    write_job,
};
pub use reply::{
    Completion, FaultCode, Frame, FrameKind, Progress, ReplyReader, ReplyTracker, ReplyWriter,
    receive_reply,
};

pub const MAX_METADATA: usize = 64 * 1024;
pub const MAX_REPLY_METADATA: usize = 1024 * 1024;
pub const MAX_DATA: usize = 64 * 1024;
pub const MAX_INPUT: u64 = 256 * 1024 * 1024;
pub const MAX_OUTPUT: u64 = 512 * 1024 * 1024;
/// All frame headers plus every non-DATA payload, independently of DATA limits.
pub const MAX_WIRE_OVERHEAD: u64 = 2 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("invalid document worker message")]
    Invalid,
    #[error("document worker message exceeds its limit")]
    Limit,
    #[error("document worker data identity mismatch")]
    Integrity,
    #[error("document worker reply order is invalid")]
    Order,
    #[error("document worker allocation refused")]
    Allocation,
    #[error("document worker refused the request: {0:?}")]
    Worker(FaultCode),
    #[error("document worker I/O failed: {0:?}")]
    Io(std::io::ErrorKind),
}
pub type ProtocolResult<T> = Result<T, ProtocolError>;

pub(crate) fn read_exact(reader: &mut dyn std::io::Read, bytes: &mut [u8]) -> ProtocolResult<()> {
    reader
        .read_exact(bytes)
        .map_err(|error| ProtocolError::Io(error.kind()))
}
pub(crate) fn write_all(writer: &mut dyn std::io::Write, bytes: &[u8]) -> ProtocolResult<()> {
    writer
        .write_all(bytes)
        .map_err(|error| ProtocolError::Io(error.kind()))
}
pub(crate) fn buffer(length: usize) -> ProtocolResult<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ProtocolError::Allocation)?;
    bytes.resize(length, 0);
    Ok(bytes)
}
pub(crate) fn json<T: serde::Serialize>(value: &T, limit: usize) -> ProtocolResult<Vec<u8>> {
    struct Bounded {
        bytes: Vec<u8>,
        limit: usize,
        exceeded: bool,
        allocation_failed: bool,
    }
    impl std::io::Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let Some(next) = self.bytes.len().checked_add(bytes.len()) else {
                self.exceeded = true;
                return Err(std::io::ErrorKind::FileTooLarge.into());
            };
            if next > self.limit {
                self.exceeded = true;
                return Err(std::io::ErrorKind::FileTooLarge.into());
            }
            if next > self.bytes.capacity() {
                let desired = next
                    .max(self.bytes.capacity().saturating_mul(2).max(256))
                    .min(self.limit);
                if self
                    .bytes
                    .try_reserve_exact(desired - self.bytes.len())
                    .is_err()
                {
                    self.allocation_failed = true;
                    return Err(std::io::ErrorKind::OutOfMemory.into());
                }
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut out = Bounded {
        bytes: Vec::new(),
        limit,
        exceeded: false,
        allocation_failed: false,
    };
    if serde_json::to_writer(&mut out, value).is_err() {
        return Err(if out.exceeded {
            ProtocolError::Limit
        } else if out.allocation_failed {
            ProtocolError::Allocation
        } else {
            ProtocolError::Invalid
        });
    }
    Ok(out.bytes)
}

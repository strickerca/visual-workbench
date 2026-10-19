//! Private, length-delimited helper pipe. Headers are untrusted data; neither
//! side treats their echoed owner/source fields as a capture grant.
use crate::{
    Error, FrameIdentity, Result,
    stream::{OfferFeedback, StreamRequest, stream_error},
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use vw_stream::{BudgetedBytes, Cancellation, MemoryCharge, MemoryKind, SessionBudget};

pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Open {
        request: StreamRequest,
    },
    Poll {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        feedback: Option<OfferFeedback>,
        repair_keyframe: bool,
    },
    Original {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        identity: FrameIdentity,
    },
    Stop {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
    },
}
impl Command {
    pub fn sequence(&self) -> Option<u64> {
        match self {
            Self::Open { .. } => None,
            Self::Poll { sequence, .. }
            | Self::Original { sequence, .. }
            | Self::Stop { sequence, .. } => Some(*sequence),
        }
    }
    pub fn binding(&self) -> Option<(u64, u64)> {
        match self {
            Self::Open { .. } => None,
            Self::Poll {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Original {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Stop {
                connection_epoch,
                source_generation,
                ..
            } => Some((*connection_epoch, *source_generation)),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case", deny_unknown_fields)]
pub enum Header {
    Ready {
        request_hash: String,
        connection_epoch: u64,
        source_generation: u64,
        ring_frames: usize,
        ring_bytes: u64,
        endpoint_bytes: u64,
        border_visible: bool,
    },
    Tiles {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        frame_id: u64,
        timestamp_ns: u64,
        captured_at_ms: i64,
        keyframe: bool,
        payload_bytes: u32,
    },
    Original {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        identity: FrameIdentity,
        asset_id: String,
        payload_bytes: u32,
    },
    Idle {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        retained_frames: usize,
        retained_bytes: u64,
    },
    Refused {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
        error: Error,
    },
    Stopped {
        sequence: u64,
        connection_epoch: u64,
        source_generation: u64,
    },
}
impl Header {
    pub fn sequence(&self) -> Option<u64> {
        match self {
            Self::Ready { .. } => None,
            Self::Tiles { sequence, .. }
            | Self::Original { sequence, .. }
            | Self::Idle { sequence, .. }
            | Self::Refused { sequence, .. }
            | Self::Stopped { sequence, .. } => Some(*sequence),
        }
    }
    pub fn binding(&self) -> (u64, u64) {
        match self {
            Self::Ready {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Tiles {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Original {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Idle {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Refused {
                connection_epoch,
                source_generation,
                ..
            }
            | Self::Stopped {
                connection_epoch,
                source_generation,
                ..
            } => (*connection_epoch, *source_generation),
        }
    }
    pub fn payload_len(&self) -> Result<usize> {
        match self {
            Self::Tiles { payload_bytes, .. }
                if *payload_bytes as usize <= vw_stream::MAX_TILE_BYTES + 64 * 1024 =>
            {
                Ok(*payload_bytes as usize)
            }
            Self::Original { payload_bytes, .. }
                if *payload_bytes as usize <= MAX_PAYLOAD_BYTES =>
            {
                Ok(*payload_bytes as usize)
            }
            Self::Tiles { .. } | Self::Original { .. } => Err(Error::Limit),
            _ => Ok(0),
        }
    }
}

pub struct Packet {
    pub header: Header,
    pub payload: BudgetedBytes,
    _header_charge: MemoryCharge,
}
pub struct ReceivedCommand {
    pub command: Command,
    _charge: MemoryCharge,
}

fn write_header<W: Write, T: Serialize>(
    output: &mut W,
    value: &T,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    let mut owned = BudgetedBytes::allocate(budget, MemoryKind::Transfer, MAX_HEADER_BYTES, cancel)
        .map_err(stream_error)?;
    let mut cursor = std::io::Cursor::new(owned.bytes_mut());
    serde_json::to_writer(&mut cursor, value).map_err(|_| Error::Limit)?;
    let size = usize::try_from(cursor.position()).map_err(|_| Error::Limit)?;
    if size == 0 || size > MAX_HEADER_BYTES {
        return Err(Error::Limit);
    }
    cancel.check().map_err(stream_error)?;
    output
        .write_all(&u32::try_from(size).map_err(|_| Error::Limit)?.to_le_bytes())
        .and_then(|()| output.write_all(&owned.bytes()[..size]))
        .map_err(|_| Error::Platform)
}
fn read_header<R: Read, T: for<'de> Deserialize<'de>>(
    input: &mut R,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<T> {
    cancel.check().map_err(stream_error)?;
    let mut len = [0u8; 4];
    input.read_exact(&mut len).map_err(|_| Error::Platform)?;
    let n = u32::from_le_bytes(len) as usize;
    if n == 0 || n > MAX_HEADER_BYTES {
        return Err(Error::Limit);
    }
    let mut owned =
        BudgetedBytes::allocate(budget, MemoryKind::Transfer, n, cancel).map_err(stream_error)?;
    input
        .read_exact(owned.bytes_mut())
        .map_err(|_| Error::Platform)?;
    cancel.check().map_err(stream_error)?;
    serde_json::from_slice(owned.bytes()).map_err(|_| Error::Invalid)
}
pub fn write_command<W: Write>(
    output: &mut W,
    command: &Command,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    write_header(output, command, budget, cancel)?;
    output.flush().map_err(|_| Error::Platform)
}
pub fn command_hash(
    command: &Command,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<String> {
    let mut owned = BudgetedBytes::allocate(budget, MemoryKind::Transfer, MAX_HEADER_BYTES, cancel)
        .map_err(stream_error)?;
    let mut cursor = std::io::Cursor::new(owned.bytes_mut());
    serde_json::to_writer(&mut cursor, command).map_err(|_| Error::Limit)?;
    let size = usize::try_from(cursor.position()).map_err(|_| Error::Limit)?;
    if size == 0 || size > MAX_HEADER_BYTES {
        return Err(Error::Limit);
    }
    Ok(vw_model::AssetId::hash(&owned.bytes()[..size]).to_string())
}
pub fn read_command<R: Read>(
    input: &mut R,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<ReceivedCommand> {
    let command = read_header(input, budget, cancel)?;
    let charge = budget
        .reserve(MemoryKind::Transfer, MAX_HEADER_BYTES as u64)
        .map_err(stream_error)?;
    Ok(ReceivedCommand {
        command,
        _charge: charge,
    })
}
pub fn write_packet<W: Write>(
    output: &mut W,
    header: &Header,
    payload: &[u8],
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<()> {
    if header.payload_len()? != payload.len() {
        return Err(Error::Invalid);
    }
    write_header(output, header, budget, cancel)?;
    cancel.check().map_err(stream_error)?;
    output
        .write_all(payload)
        .and_then(|()| output.flush())
        .map_err(|_| Error::Platform)
}
pub fn read_packet<R: Read>(
    input: &mut R,
    budget: &SessionBudget,
    cancel: &Cancellation,
) -> Result<Packet> {
    let header: Header = read_header(input, budget, cancel)?;
    let header_charge = budget
        .reserve(MemoryKind::Transfer, MAX_HEADER_BYTES as u64)
        .map_err(stream_error)?;
    let len = header.payload_len()?;
    let mut payload =
        BudgetedBytes::allocate(budget, MemoryKind::Transfer, len, cancel).map_err(stream_error)?;
    input
        .read_exact(payload.bytes_mut())
        .map_err(|_| Error::Platform)?;
    cancel.check().map_err(stream_error)?;
    Ok(Packet {
        header,
        payload,
        _header_charge: header_charge,
    })
}

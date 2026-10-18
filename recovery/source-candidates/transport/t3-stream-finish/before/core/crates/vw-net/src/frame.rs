use crate::{NetError, Result};
use vw_proto::{
    Message,
    v1::{self, envelope::Body},
};

pub const HEADER_BYTES: usize = 12;
const MAGIC: &[u8; 4] = b"VWM1";

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub channel: v1::Channel,
    pub envelope: v1::Envelope,
}

pub const fn payload_limit(channel: v1::Channel) -> usize {
    match channel {
        v1::Channel::Control => 64 * 1024,
        v1::Channel::Ops => 8 * 1024 * 1024,
        v1::Channel::Ephemeral => 1100,
        v1::Channel::Blob => 128 * 1024,
        v1::Channel::Media => 8 * 1024 * 1024,
        v1::Channel::Input => 4096,
        v1::Channel::Unspecified => 0,
    }
}
pub fn channel_for(body: &Body) -> v1::Channel {
    match body {
        Body::Txn(_)
        | Body::TxnAck(_)
        | Body::TxnReject(_)
        | Body::SyncRequest(_)
        | Body::SyncBatch(_)
        | Body::RebaseNotice(_)
        | Body::ConflictNotice(_) => v1::Channel::Ops,
        Body::GestureUpdate(_)
        | Body::GestureCancel(_)
        | Body::CursorUpdate(_)
        | Body::ViewportOutline(_) => v1::Channel::Ephemeral,
        Body::BlobRequest(_)
        | Body::BlobChunk(_)
        | Body::BlobAck(_)
        | Body::TileRequest(_)
        | Body::Tile(_)
        | Body::LosslessFrame(_) => v1::Channel::Blob,
        Body::FrameTiles(_) | Body::VideoFrame(_) => v1::Channel::Media,
        Body::InputEvent(_) | Body::InputStatus(_) => v1::Channel::Input,
        _ => v1::Channel::Control,
    }
}
fn flags(channel: v1::Channel) -> u8 {
    u8::from(matches!(
        channel,
        v1::Channel::Ephemeral | v1::Channel::Media
    ))
}
pub(crate) fn read_header(header: &[u8; HEADER_BYTES]) -> Result<(v1::Channel, usize)> {
    if &header[..4] != MAGIC || header[6..8] != [0, 0] {
        return Err(NetError::Invalid("frame header"));
    }
    let channel =
        v1::Channel::try_from(i32::from(header[4])).map_err(|_| NetError::Invalid("channel"))?;
    let length = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if header[5] != flags(channel) || length == 0 || length > payload_limit(channel) {
        return Err(NetError::Invalid("frame quota"));
    }
    Ok((channel, length))
}
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>> {
    let body = frame
        .envelope
        .body
        .as_ref()
        .ok_or(NetError::Invalid("missing body"))?;
    let len = frame.envelope.encoded_len();
    if frame.envelope.channel != frame.channel as i32
        || channel_for(body) != frame.channel
        || len == 0
        || len > payload_limit(frame.channel)
    {
        return Err(NetError::Invalid("body channel/quota"));
    }
    let mut out = Vec::with_capacity(HEADER_BYTES + len);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[frame.channel as u8, flags(frame.channel), 0, 0]);
    out.extend_from_slice(&(len as u32).to_be_bytes());
    frame
        .envelope
        .encode(&mut out)
        .map_err(|_| NetError::Invalid("protobuf encode"))?;
    Ok(out)
}
pub fn decode_frame(bytes: &[u8]) -> Result<Frame> {
    let header: &[u8; HEADER_BYTES] = bytes
        .get(..HEADER_BYTES)
        .ok_or(NetError::Invalid("short frame"))?
        .try_into()
        .map_err(|_| NetError::Invalid("header"))?;
    let (channel, len) = read_header(header)?;
    if bytes.len() != HEADER_BYTES + len {
        return Err(NetError::Invalid("frame length"));
    }
    let envelope = v1::Envelope::decode(&bytes[HEADER_BYTES..])
        .map_err(|_| NetError::Invalid("protobuf decode"))?;
    if envelope.channel != channel as i32
        || channel_for(
            envelope
                .body
                .as_ref()
                .ok_or(NetError::Invalid("missing body"))?,
        ) != channel
    {
        return Err(NetError::Invalid("body channel"));
    }
    Ok(Frame { channel, envelope })
}

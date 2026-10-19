//! Actual codec/packet/foreign allocations share an endpoint and process pool.
use crate::session::{SessionError, SessionResult};
use std::sync::{Arc, OnceLock};
use vw_stream::{MemoryCharge, MemoryKind, SessionBudget};

pub(super) const MAX_RECEIVER_BYTES: u64 = 512 * 1024 * 1024;
static PROCESS: OnceLock<SessionBudget> = OnceLock::new();

#[derive(Clone)]
pub(super) struct ByteBudget(SessionBudget);
pub(super) struct Bytes {
    budget: ByteBudget,
    _charge: MemoryCharge,
}
impl ByteBudget {
    pub fn new(limit: u64) -> SessionResult<Self> {
        if limit == 0 || limit > MAX_RECEIVER_BYTES {
            return Err(SessionError::Invalid);
        }
        let process = match PROCESS.get() {
            Some(pool) => pool,
            None => {
                let pool =
                    SessionBudget::new(MAX_RECEIVER_BYTES).map_err(super::decoder::map_stream)?;
                let _ = PROCESS.set(pool);
                PROCESS.get().ok_or(SessionError::Worker)?
            }
        };
        Ok(Self(
            process.child(limit).map_err(super::decoder::map_stream)?,
        ))
    }
    pub fn reserve(&self, bytes: u64) -> SessionResult<Bytes> {
        let charge = self
            .0
            .reserve(MemoryKind::Foreign, bytes)
            .map_err(super::decoder::map_stream)?;
        Ok(Bytes {
            budget: self.clone(),
            _charge: charge,
        })
    }
    pub fn pool(&self) -> &SessionBudget {
        &self.0
    }
    pub fn used(&self) -> u64 {
        self.0.used_bytes()
    }
}

/// The actual pixel Arc contains its linear core allocation charge. This
/// wrapper retains metadata and endpoint identity, never nominal pixel credit.
pub(super) struct OwnedFrame {
    pub(super) frame: Arc<vw_stream::DisplayFrame>,
    stamp: super::wire::CaptureStamp,
    _bytes: Bytes,
}
impl OwnedFrame {
    pub fn new(
        frame: vw_stream::DisplayFrame,
        stamp: super::wire::CaptureStamp,
        bytes: Bytes,
    ) -> SessionResult<Arc<Self>> {
        if !frame.belongs_to(bytes.budget.pool()) {
            return Err(SessionError::Invalid);
        }
        Ok(Arc::new(Self {
            frame: Arc::new(frame),
            stamp,
            _bytes: bytes,
        }))
    }
    pub fn pixels(&self) -> &[u8] {
        self.frame.rgba()
    }
    pub fn identity(&self) -> &vw_stream::FrameIdentity {
        self.frame.identity()
    }
    pub fn stamp(&self) -> &super::wire::CaptureStamp {
        &self.stamp
    }
    pub fn belongs_to(&self, budget: &ByteBudget) -> bool {
        self.frame.belongs_to(budget.pool())
    }
}

impl Bytes {
    pub fn into_charge(self) -> MemoryCharge {
        self._charge
    }
}

/// Reserve both the retained DTO and two possible simultaneous encoded buffers
/// before network admission. The sender retains this through actual carrier IO.
pub(super) fn enqueue(
    sender: &vw_net::ConnectionSender,
    budget: &ByteBudget,
    body: vw_proto::v1::envelope::Body,
) -> vw_net::Result<()> {
    use vw_proto::{Message, v1::envelope::Body};
    let wire = match &body {
        Body::FrameTiles(v) => v.encoded_len(),
        Body::StreamBlob(v) => v.encoded_len(),
        Body::LosslessFrame(v) => v.encoded_len(),
        Body::StreamControl(v) => v.encoded_len(),
        Body::FreezeRequest(v) => v.encoded_len(),
        Body::RegionCaptureControl(v) => v.encoded_len(),
        _ => return Err(vw_net::NetError::Invalid("capture queue body")),
    } as u64;
    let retained = match &body {
        Body::FrameTiles(packet) => {
            super::wire::packet_capacity(packet).map_err(|_| vw_net::NetError::Backpressure)?
        }
        Body::StreamBlob(v) => match &v.body {
            Some(vw_proto::v1::stream_blob::Body::Chunk(chunk)) => {
                chunk.data.capacity() as u64 + 8192
            }
            _ => wire + 8192,
        },
        _ => wire + 8192,
    };
    let bytes = retained
        .checked_add(
            wire.checked_add(1024)
                .and_then(|v| v.checked_mul(2))
                .ok_or(vw_net::NetError::Backpressure)?,
        )
        .ok_or(vw_net::NetError::Backpressure)?;
    let reservation = budget
        .reserve(bytes)
        .map_err(|_| vw_net::NetError::Backpressure)?;
    sender.enqueue_retained(body, Box::new(reservation))
}

//! Bounded authenticated transport, durable OPS delivery and independent channel
//! recovery. Trust provisioning is supplied by T1.06b; no accept-any TLS mode.
mod blob;
pub mod carrier;
mod carrier_authorization;
mod connection;
pub mod discovery;
mod frame;
mod ingress;
pub mod pairing;
mod replay;
pub mod route;
mod session;
mod sync;
pub use blob::{BLOB_CHUNK_BYTES, BLOB_WINDOW_BYTES, BlobReceiver, BlobSender};
pub use carrier::AuthenticatedPeer;
pub use connection::{CarrierIo, ConnectionReceiver, ConnectionSender, SecureConnection};
pub use frame::{Frame, HEADER_BYTES, channel_for, decode_frame, encode_frame, payload_limit};
pub use session::{
    ConnectionEvent, ConnectionMachine, ConnectionState, InputGuard, LocalHello, Receive, Session,
};
pub use sync::{HostSync, SyncReceiver, SyncResponse};

pub const PROTOCOL_MAJOR: u32 = 1;
pub const PROTOCOL_MINOR: u32 = 4;
pub const CARRIER_ORDER: [vw_proto::v1::Carrier; 3] = [
    vw_proto::v1::Carrier::QuicTether,
    vw_proto::v1::Carrier::QuicWifi,
    vw_proto::v1::Carrier::TcpAdb,
];

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("invalid transport field: {0}")]
    Invalid(&'static str),
    #[error("channel queue full; durable work remains pending")]
    Backpressure,
    #[error("latest frame replacement requires a fresh independently decodable keyframe")]
    KeyframeRequired,
    #[error("peer authentication or TLS configuration failed")]
    Authentication,
    #[error("bounded transport operation timed out")]
    Timeout,
    #[error("carrier connection failed")]
    Carrier,
    #[error("synchronization base differs; request a verified checkpoint")]
    Resync,
    #[error("content integrity mismatch")]
    Integrity,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Model(#[from] vw_model::ModelError),
    #[error(transparent)]
    Ops(#[from] vw_ops::OpsError),
    #[error(transparent)]
    Store(#[from] vw_store::StoreError),
}
pub type Result<T> = std::result::Result<T, NetError>;

#[cfg(test)]
mod focus_protocol_tests;
#[cfg(test)]
mod tests;

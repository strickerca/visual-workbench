//! Draft wire types generated from the shared protobuf contract.
//! Model validation is required after decoding. These types do not establish
//! protocol compatibility, authentication or transport acceptance (T1.06a/b).

/// Protobuf v1 messages. Unknown fields are discarded by prost, so a peer must
/// never re-encode a message from a newer negotiated minor version.
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/vw.v1.rs"));
}

pub use prost::Message;

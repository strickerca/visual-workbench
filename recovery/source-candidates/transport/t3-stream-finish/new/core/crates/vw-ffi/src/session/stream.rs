//! capture_stream_v1: authenticated driver bridge. No API in this module grants
//! remote input, chooses an OS source, or installs a transient blob as an asset.
mod api;
pub(in crate::session) mod draft;
mod decoder;
mod driver;
pub(in crate::session) mod hub;
mod incoming;
mod lossless;
mod memory;
mod order;
mod outgoing;
mod presentation;
mod publication;
mod publication_api;
mod publish_order;
mod publish_worker;
mod receiver;
mod region;
pub use region::CaptureRegionSignal;
mod transfer;
mod verified;
mod wire;

pub use api::{CaptureBackground,CaptureDraw,CaptureStream,StreamFrameInfo,StreamFreezeReceipt,StreamSignal,StreamStage};
pub use publication_api::{CaptureOriginalRequest,CapturePublicationSignal,CapturePublisher,CapturePublishSource};
pub use draft::{CaptureDraft,CaptureDraftInfo,CaptureDraftOptions,CaptureDraftSave,CaptureDraftSaveInfo};
pub(super) use driver::Bridge;
pub(super) use hub::Hub;

#[cfg(test)]
mod order_tests;
#[cfg(test)]
mod receiver_tests;
#[cfg(test)]
mod verified_tests;

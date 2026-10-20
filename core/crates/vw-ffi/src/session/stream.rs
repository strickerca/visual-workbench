//! capture_stream_v1: authenticated driver bridge. No API in this module grants
//! remote input, chooses an OS source, or installs a transient blob as an asset.
mod api;
mod decoder;
pub(in crate::session) mod draft;
mod driver;
pub(in crate::session) mod hub;
mod incoming;
mod local_producer;
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

pub use api::{
    CaptureBackground, CaptureDraw, CaptureStream, StreamFrameInfo, StreamFreezeReceipt,
    StreamSignal, StreamStage,
};
pub use draft::{
    CaptureDraft, CaptureDraftInfo, CaptureDraftOptions, CaptureDraftSave, CaptureDraftSaveInfo,
};
pub(super) use driver::Bridge;
pub(super) use hub::Hub;
pub use local_producer::{
    CaptureLocalMonitor, CaptureLocalOptions, CaptureLocalProducer, CaptureLocalProgress,
    CaptureLocalRect, CaptureLocalSelection, CaptureLocalWindow,
};
pub use publication_api::{
    CaptureOriginalRequest, CapturePublicationSignal, CapturePublishSource, CapturePublisher,
};

#[cfg(test)]
mod order_tests;
#[cfg(test)]
mod receiver_tests;
#[cfg(test)]
mod verified_tests;

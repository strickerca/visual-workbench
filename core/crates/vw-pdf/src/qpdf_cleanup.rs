//! qpdf12.4.2 C ABI in the same bounded Windows document worker. qpdf cleans
//! only our fresh generated document; it is never the redaction authority.
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::QpdfRuntime;

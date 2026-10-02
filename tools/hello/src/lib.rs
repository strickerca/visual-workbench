//! Setup-only toolchain smoke library. No application features are implemented.

/// Returns the shared core's setup smoke ABI version.
///
// SAFETY: This project-specific symbol has one definition in this library.
// The called core function takes no pointers, allocates nothing and cannot unwind.
#[unsafe(no_mangle)]
pub extern "C" fn vw_hello_toolchain_smoke_version() -> u32 {
    vw_core::vw_toolchain_smoke_version()
}

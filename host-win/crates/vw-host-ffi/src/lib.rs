//! Future module stub for Windows host FFI.
//! Product behavior is implemented and verified by the owning later task.

#![cfg(target_os = "windows")]

/// Returns the Windows host setup smoke ABI version only.
///
// SAFETY: This project-specific symbol has one definition in this library,
// takes no pointers, allocates nothing and cannot unwind across the C boundary.
#[unsafe(no_mangle)]
pub extern "C" fn vw_host_toolchain_smoke_version() -> u32 {
    1
}

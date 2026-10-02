//! Future module stub for shared core FFI.
//! Product behavior is implemented and verified by the owning later task.

/// Returns the setup smoke ABI version. This exposes no product functionality.
///
// SAFETY: This project-specific symbol has one definition in this library,
// takes no pointers, allocates nothing and cannot unwind across the C boundary.
#[unsafe(no_mangle)]
pub extern "C" fn vw_toolchain_smoke_version() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke_abi_is_callable_through_c_function_pointer() {
        let entry: extern "C" fn() -> u32 = super::vw_toolchain_smoke_version;
        assert_eq!(entry(), 1);
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Cargo target variables preserve Android cross-compilation on Windows hosts.
    let target_os = std::env::var("CARGO_CFG_TARGET_OS");
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV");
    if matches!(target_os.as_deref(), Ok("windows")) && matches!(target_env.as_deref(), Ok("msvc"))
    {
        // Only vw_core's cdylib link omits its PDB; tests, rlib and bins keep theirs.
        println!("cargo:rustc-link-arg-cdylib=/DEBUG:NONE");
    }
}

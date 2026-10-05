//! Explicit owner-ready harness. Never run by ordinary test targets.
fn main() {
    #[cfg(windows)]
    let result = vw_remote_host::hil::run();
    #[cfg(not(windows))]
    let result: vw_remote_host::Result<()> = Err(vw_remote_host::Error::Unavailable);
    if let Err(error) = result {
        eprintln!("remote-guard-hil: {error}");
        std::process::exit(1);
    }
}

//! Explicit read-only selected-editor measurement; never an ordinary test action.
fn main() {
    #[cfg(windows)]
    let result = vw_remote_host::editor_probe::run();
    #[cfg(not(windows))]
    let result: vw_remote_host::Result<()> = Err(vw_remote_host::Error::Unavailable);
    if let Err(error) = result {
        eprintln!("editor-probe: {error}");
        std::process::exit(1);
    }
}

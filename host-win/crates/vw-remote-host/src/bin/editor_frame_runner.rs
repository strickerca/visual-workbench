fn main() {
    #[cfg(windows)]
    let result = vw_remote_host::editor_probe_runner::run_editor_frame();
    #[cfg(not(windows))]
    let result: vw_remote_host::Result<()> = Err(vw_remote_host::Error::Unavailable);
    if let Err(error) = result {
        eprintln!("editor-frame-runner: {error}");
        #[cfg(windows)]
        vw_remote_host::editor_probe_runner::wait_editor_frame_retained();
        std::process::exit(1);
    }
}

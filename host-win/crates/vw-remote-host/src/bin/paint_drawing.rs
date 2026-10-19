fn main() {
    #[cfg(windows)]
    let result = vw_remote_host::editor_probe_runner::run_paint_drawing_helper();
    #[cfg(not(windows))]
    let result: vw_remote_host::Result<()> = Err(vw_remote_host::Error::Unavailable);
    if let Err(error) = result {
        eprintln!("paint-drawing-helper: {error}");
        std::process::exit(1);
    }
}

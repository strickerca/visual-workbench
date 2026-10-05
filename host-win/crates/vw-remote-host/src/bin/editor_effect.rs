fn main() {
    #[cfg(windows)]
    let result = vw_remote_host::editor_effect::run();
    #[cfg(not(windows))]
    let result: vw_remote_host::Result<()> = Err(vw_remote_host::Error::Unavailable);
    if let Err(error) = result {
        eprintln!("editor-effect-hil: {error}");
        std::process::exit(1);
    }
}

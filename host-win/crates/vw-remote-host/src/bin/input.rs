fn main() {
    #[cfg(windows)]
    {
        if vw_remote_host::platform::run_input().is_err() {
            std::process::exit(2)
        }
    }
    #[cfg(not(windows))]
    {
        std::process::exit(2)
    }
}

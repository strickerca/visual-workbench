fn main() {
    #[cfg(windows)]
    {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let result = if args.len() == 2 {
            /* SAFETY: this dedicated binary has started no threads or other libraries. */
            unsafe {
                vw_assets_win::helper(
                    std::path::Path::new(&args[0]),
                    std::path::Path::new(&args[1]),
                )
            }
        } else {
            Err(vw_assets_win::Error::Invalid)
        };
        match result {
            Ok(()) => (),
            Err(_) => std::process::exit(2),
        }
    }
    #[cfg(not(windows))]
    {
        std::process::exit(3);
    }
}

#[cfg(windows)]
mod platform;

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    match platform::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("vdd-probe: {error}");
            std::process::ExitCode::FAILURE
        }
    }
    #[cfg(not(windows))]
    {
        eprintln!("vdd-probe requires Windows");
        std::process::ExitCode::FAILURE
    }
}

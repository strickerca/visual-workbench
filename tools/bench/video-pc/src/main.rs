#[cfg(windows)]
mod platform;
#[cfg(windows)]
fn main() {
    if let Err(error) = platform::run() {
        eprintln!("video-pc: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(windows))]
fn main() {
    eprintln!("video-pc requires Windows 11 build 26100 or newer");
    std::process::exit(1);
}

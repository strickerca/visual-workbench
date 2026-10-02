#[cfg(windows)]
mod platform;
fn main() {
    #[cfg(windows)]
    let result = platform::run();
    #[cfg(not(windows))]
    let result: Result<(), String> = Err("Windows desktop required".into());
    if let Err(error) = result {
        eprintln!("pen-harness: {error}");
        std::process::exit(1);
    }
}

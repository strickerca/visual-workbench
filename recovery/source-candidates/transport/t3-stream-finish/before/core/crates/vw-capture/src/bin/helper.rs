use std::io::{Read, Write};
use vw_capture::*;
fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    // One watchdog for the whole owned process, including blocked stdin, GPU,
    // foreign UIA providers and a parent that stopped draining private stdout.
    let start = std::time::Instant::now();
    let deadline = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(8000));
    let watch = deadline.clone();
    std::thread::Builder::new()
        .name("vw-capture-deadline".into())
        .spawn(move || {
            loop {
                if start.elapsed().as_millis()
                    > u128::from(watch.load(std::sync::atomic::Ordering::Acquire))
                {
                    std::process::exit(2);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        })
        .map_err(|_| Error::Platform)?;
    let mut request = Vec::new();
    std::io::stdin()
        .take(16 * 1024 + 1)
        .read_to_end(&mut request)
        .map_err(|_| Error::Platform)?;
    if request.len() > 16 * 1024 {
        return Err(Error::Limit);
    }
    let request: Request = serde_json::from_slice(&request).map_err(|_| Error::Invalid)?;
    let remaining = match &request {
        Request::Capture(value) => value.limits.validate()?.capture_ms + 1000,
        Request::Tree(value) => value.limits.validate()?.tree_ms + 50,
    };
    deadline.store(
        u64::try_from(start.elapsed().as_millis()).map_err(|_| Error::Limit)? + remaining,
        std::sync::atomic::Ordering::Release,
    );
    #[cfg(windows)]
    let _job = process::confine_helper()?;
    #[cfg(windows)]
    let response = {
        // SAFETY: helper process owns this MTA until its synchronous work ends.
        unsafe {
            ::windows::Win32::System::WinRT::RoInitialize(
                ::windows::Win32::System::WinRT::RO_INIT_MULTITHREADED,
            )
            .map_err(|_| Error::Platform)?;
        }
        let cancel = Cancellation::default();
        let result = match request {
            Request::Capture(value) => windows::capture(value, &cancel).map(Response::Frame),
            Request::Tree(value) => windows::collect(value, &cancel).map(Response::Tree),
        };
        // SAFETY: paired successful RoInitialize on this thread.
        unsafe {
            ::windows::Win32::System::WinRT::RoUninitialize();
        }
        result.unwrap_or_else(|error| Response::Refused { error })
    };
    #[cfg(not(windows))]
    let response = {
        let _ = request;
        Response::Refused {
            error: Error::Unsupported,
        }
    };
    struct Capped {
        bytes: Vec<u8>,
    }
    impl Write for Capped {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            if self.bytes.len().saturating_add(value.len()) > 4 * 1024 * 1024 {
                return Err(std::io::Error::other("limit"));
            }
            self.bytes.extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Capped { bytes: Vec::new() };
    serde_json::to_writer(&mut output, &response).map_err(|_| Error::Limit)?;
    std::io::stdout()
        .write_all(&output.bytes)
        .and_then(|()| std::io::stdout().flush())
        .map_err(|_| Error::Platform)?;
    Ok(())
}

//! Generate and independently decode a synthetic 1-second MP4 fixture with
//! Windows Media Foundation. No desktop capture, external media or encoder tool.
#[cfg(windows)]
#[path = "support/video_platform.rs"]
mod platform;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let path = std::env::args_os().nth(1).ok_or("output path required")?;
        platform::generate(std::path::Path::new(&path))?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err("fixture generation uses Windows Media Foundation".into())
}

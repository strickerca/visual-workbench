use crate::*;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};
pub fn plain(path: &Path, directory: bool) -> Result<()> {
    if !path.is_absolute() {
        return Err(Error::Invalid);
    }
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir | std::path::Component::CurDir
        ) {
            return Err(Error::Invalid);
        }
        current.push(component);
        if matches!(component, std::path::Component::Prefix(_)) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&current).map_err(|_| Error::Storage)?;
        if metadata.file_type().is_symlink() {
            return Err(Error::Storage);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(Error::Storage);
            }
        }
    }
    let metadata = std::fs::metadata(path).map_err(|_| Error::Storage)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        Err(Error::Storage)
    } else {
        Ok(())
    }
}
struct Bounded<'a> {
    file: &'a mut File,
    hash: blake3::Hasher,
    count: u64,
    limit: u64,
}
impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.count.saturating_add(bytes.len() as u64) > self.limit {
            return Err(io::Error::other("encoded limit"));
        }
        let n = self.file.write(bytes)?;
        self.hash.update(&bytes[..n]);
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
pub fn publish_png(
    directory: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    limits: Limits,
    cancel: &Cancellation,
) -> Result<(String, u64)> {
    publish_named_png(
        directory,
        width,
        height,
        rgba,
        limits,
        cancel,
        "capture.png",
    )
}
#[cfg(windows)]
pub(crate) fn publish_canvas_png(
    directory: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    limits: Limits,
    cancel: &Cancellation,
) -> Result<(String, u64)> {
    publish_named_png(
        directory,
        width,
        height,
        rgba,
        limits,
        cancel,
        "capture-canvas.png",
    )
}
fn publish_named_png(
    directory: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    limits: Limits,
    cancel: &Cancellation,
    filename: &str,
) -> Result<(String, u64)> {
    if rgba.len() != limits.image(width, height)? {
        return Err(Error::Invalid);
    }
    plain(directory, true)?;
    cancel.check()?;
    let path = directory.join(filename);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0);
    }
    let mut file = options.open(&path).map_err(|_| Error::Storage)?;
    let result = (|| {
        let mut output = Bounded {
            file: &mut file,
            hash: blake3::Hasher::new(),
            count: 0,
            limit: limits.png_bytes,
        };
        {
            let mut encoder = png::Encoder::new(&mut output, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
            let mut writer = encoder.write_header().map_err(|_| Error::Storage)?;
            let mut stream = writer.stream_writer().map_err(|_| Error::Storage)?;
            for row in rgba.chunks_exact(width as usize * 4) {
                cancel.check()?;
                stream.write_all(row).map_err(|_| Error::Storage)?;
            }
            stream.finish().map_err(|_| Error::Storage)?;
            writer.finish().map_err(|_| Error::Storage)?;
        }
        output.flush().map_err(|_| Error::Storage)?;
        let receipt = (output.hash.finalize().to_hex().to_string(), output.count);
        drop(output);
        file.sync_all().map_err(|_| Error::Storage)?;
        cancel.check()?;
        Ok(receipt)
    })();
    drop(file);
    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}

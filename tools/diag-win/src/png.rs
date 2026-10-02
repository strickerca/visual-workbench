pub const MAX_PIXELS: usize = 32_000_000;

pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for byte in bytes {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn chunk(output: &mut Vec<u8>, kind: &[u8; 4], contents: &[u8]) {
    output.extend_from_slice(&(contents.len() as u32).to_be_bytes());
    let start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(contents);
    output.extend_from_slice(&crc32(&output[start..]).to_be_bytes());
}

pub fn rgb(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, String> {
    let count = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Screenshot dimensions overflow")?;
    if count == 0 || count > MAX_PIXELS || pixels.len() != count * 3 {
        return Err("Screenshot dimensions or RGB byte count invalid".into());
    }
    let stride = width as usize * 3;
    let mut scanlines = Vec::with_capacity(pixels.len() + height as usize);
    for row in pixels.chunks_exact(stride) {
        scanlines.push(0);
        scanlines.extend_from_slice(row);
    }
    // PNG's zlib stream, using bounded uncompressed DEFLATE blocks.
    let mut zlib = vec![0x78, 0x01];
    let blocks = scanlines.chunks(65535);
    let block_count = blocks.len();
    for (index, block) in blocks.enumerate() {
        zlib.push(u8::from(index + 1 == block_count));
        let length = block.len() as u16;
        zlib.extend_from_slice(&length.to_le_bytes());
        zlib.extend_from_slice(&(!length).to_le_bytes());
        zlib.extend_from_slice(block);
    }
    zlib.extend_from_slice(&adler32(&scanlines).to_be_bytes());
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut output, b"IHDR", &header);
    chunk(&mut output, b"IDAT", &zlib);
    chunk(&mut output, b"IEND", &[]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    #[test]
    fn checksums_match_standard_vectors() {
        assert_eq!(super::crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(super::adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn invalid_or_unbounded_images_are_rejected() {
        assert!(super::rgb(0, 1, &[]).is_err());
        assert!(super::rgb(2, 2, &[0; 3]).is_err());
        assert!(super::rgb(32001, 1000, &[]).is_err());
    }

    #[test]
    fn png_header_and_terminator_are_correct() -> Result<(), String> {
        let image = super::rgb(1, 1, &[255, 0, 128])?;
        assert_eq!(&image[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&image[16..24], &[0, 0, 0, 1, 0, 0, 0, 1]);
        assert_eq!(&image[image.len() - 8..image.len() - 4], b"IEND");
        Ok(())
    }
}

use crate::{Cancellation, Error, ImageMapping, Result, Target, bounded, check};
use vw_instructions::Role;
use vw_raster::{DecodedImage, Pixels};

pub(crate) struct Frame {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}
/// Rational area resampling in premultiplied alpha. Integer arithmetic makes
/// pixels identical on both architectures. One uniform scale covers D geometry;
/// any fractional last row/column and minimum-side padding are white context.
pub(crate) fn resample(
    source: &DecodedImage,
    map: &ImageMapping,
    budget: u64,
    cancel: &dyn Cancellation,
) -> Result<Frame> {
    let Pixels::Rgba8(input) = &source.pixels else {
        return Err(Error::Invalid("sRGB derivative depth"));
    };
    let length = u64::from(map.width) * u64::from(map.height) * 4;
    // The same frame is then encoded: retain room for PNG row/filter/compressor
    // buffers as well as the separately reserved entire package output.
    if length
        .checked_add(32 * 1024 * 1024)
        .is_none_or(|n| n > budget)
    {
        return Err(Error::Limit("resample memory"));
    }
    let work = (map.source_rectangle[2] as u64 + u64::from(map.content_width))
        * (map.source_rectangle[3] as u64 + u64::from(map.content_height));
    if work > 220_000_000 {
        return Err(Error::Limit("resample work"));
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(usize::try_from(length).map_err(|_| Error::Limit("image size"))?)
        .map_err(|_| Error::Limit("image allocation"))?;
    bytes.resize(length as usize, 255);
    let n = u64::from(map.scale_numerator);
    let d = u64::from(map.scale_denominator);
    let area = d * d;
    for y in 0..map.content_height {
        check(cancel)?;
        let y0 = u64::from(y) * d;
        let y1 = y0 + d;
        for x in 0..map.content_width {
            let x0 = u64::from(x) * d;
            let x1 = x0 + d;
            let mut sums = [0u64; 4];
            for sy in y0 / n..y1.div_ceil(n) {
                let wy = y1.min((sy + 1) * n) - y0.max(sy * n);
                for sx in x0 / n..x1.div_ceil(n) {
                    let wx = x1.min((sx + 1) * n) - x0.max(sx * n);
                    let weight = wx * wy;
                    let px = map.source_rectangle[0] + sx as i64;
                    let py = map.source_rectangle[1] + sy as i64;
                    let pixel = if sx < map.source_rectangle[2] as u64
                        && sy < map.source_rectangle[3] as u64
                        && px >= 0
                        && py >= 0
                        && px < i64::from(source.width)
                        && py < i64::from(source.height)
                    {
                        let at = (py as usize * source.width as usize + px as usize) * 4;
                        input
                            .get(at..at + 4)
                            .ok_or(Error::Invalid("pixel buffer"))?
                    } else {
                        &[255, 255, 255, 255]
                    };
                    let alpha = u64::from(pixel[3]);
                    sums[3] += alpha * weight;
                    for c in 0..3 {
                        sums[c] += u64::from(pixel[c]) * alpha * weight;
                    }
                }
            }
            let at = (y as usize * map.width as usize + x as usize) * 4;
            bytes[at + 3] = ((sums[3] + area / 2) / area) as u8;
            for c in 0..3 {
                bytes[at + c] = (sums[c] + sums[3] / 2).checked_div(sums[3]).unwrap_or(0) as u8;
            }
        }
    }
    Ok(Frame {
        width: map.width,
        height: map.height,
        bytes,
    })
}
fn color(role: Role) -> [u8; 4] {
    match role {
        Role::Change => [240, 60, 50, 255],
        Role::Preserve => [35, 170, 80, 255],
        Role::Reference => [35, 120, 235, 255],
        Role::Explain => [165, 75, 230, 255],
        Role::None => [110, 110, 110, 255],
    }
}
impl Frame {
    fn fill(&mut self, x: i64, y: i64, w: i64, h: i64, c: [u8; 4]) {
        for row in y.max(0)..(y + h).min(i64::from(self.height)) {
            for col in x.max(0)..(x + w).min(i64::from(self.width)) {
                let at = (row as usize * self.width as usize + col as usize) * 4;
                self.bytes[at..at + 4].copy_from_slice(&c);
            }
        }
    }
    pub fn marker(&mut self, number: u32, role: Role, bounds: [f64; 4], map: &ImageMapping) {
        let b = map.bounds(
            bounds,
            &Target::Generic {
                max_long_edge: 8192,
            },
        );
        let x = b[0].floor() as i64;
        let y = b[1].floor() as i64;
        let w = b[2].ceil().max(1.0) as i64;
        let h = b[3].ceil().max(1.0) as i64;
        // Draw inside the box so a source-edge marker keeps all three pixels.
        let c = color(role);
        self.fill(x, y, w.max(3), 3, c);
        self.fill(x, y, 3, h.max(3), c);
        self.fill(x, y + h - 3, w.max(3), 3, c);
        self.fill(x + w - 3, y, 3, h.max(3), c);
        let digits = number.to_string();
        let badge_w = (digits.len() * 16 + 4) as i64;
        let bx = x.clamp(0, (i64::from(self.width) - badge_w).max(0));
        let by = (y - 24).clamp(0, (i64::from(self.height) - 24).max(0));
        self.fill(bx, by, badge_w, 24, [255, 255, 255, 255]);
        self.fill(bx + 1, by + 1, badge_w - 2, 22, c);
        // Fixed 3x5 bitmap digits at four pixels/cell; no platform font variation.
        const DIGITS: [u16; 10] = [
            0b111_101_101_101_111,
            0b010_110_010_010_111,
            0b111_001_111_100_111,
            0b111_001_111_001_111,
            0b101_101_111_001_001,
            0b111_100_111_001_111,
            0b111_100_111_101_111,
            0b111_001_010_010_010,
            0b111_101_111_101_111,
            0b111_101_111_001_111,
        ];
        for (i, ch) in digits.bytes().enumerate() {
            let bits = DIGITS[(ch - b'0') as usize];
            for row in 0..5 {
                for col in 0..3 {
                    if bits & (1 << (14 - row * 3 - col)) != 0 {
                        self.fill(
                            bx + 2 + i as i64 * 16 + col * 4,
                            by + 2 + row * 4,
                            4,
                            4,
                            [0, 0, 0, 255],
                        );
                    }
                }
            }
        }
    }
}
pub(crate) fn encode(
    frame: &Frame,
    profile: Option<&[u8]>,
    metadata: &str,
    limit: usize,
) -> Result<Vec<u8>> {
    let mut info = png::Info::with_size(frame.width, frame.height);
    info.color_type = png::ColorType::Rgba;
    info.bit_depth = png::BitDepth::Eight;
    if let Some(p) = profile {
        info.icc_profile = Some(std::borrow::Cow::Borrowed(p));
    }
    let mut out = bounded::Buffer::new(limit);
    {
        let mut encoder = png::Encoder::with_info(&mut out, info).map_err(|_| Error::Encoding)?;
        encoder.set_compression(png::Compression::Balanced);
        encoder.set_filter(png::Filter::Paeth);
        encoder
            .add_itxt_chunk("VisualWorkbenchPackage".into(), metadata.into())
            .map_err(|_| Error::Encoding)?;
        let mut writer = encoder.write_header().map_err(|_| Error::Encoding)?;
        writer
            .write_image_data(&frame.bytes)
            .map_err(|_| Error::Encoding)?;
        writer.finish().map_err(|_| Error::Encoding)?;
    }
    Ok(out.bytes)
}

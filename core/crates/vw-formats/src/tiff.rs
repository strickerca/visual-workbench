use crate::admission::alloc;
use crate::{Error, Header, Limits, Result, check};
use vw_raster::{DecodedImage, Pixels};

pub(crate) fn decode(
    bytes: &[u8],
    h: &Header,
    limits: Limits,
    cancel: &dyn Fn() -> bool,
) -> Result<Pixels> {
    limits.admit(
        h.width,
        h.height,
        bytes.len() as u64,
        h.icc.as_ref().map_or(0, Vec::len),
    )?;
    let opened = ::tiff::decoder::Decoder::new(crate::io::Input::new(bytes, cancel));
    check(cancel)?;
    let mut decoder = opened.map_err(|_| Error::Codec)?;
    let n = h.width as usize * h.height as usize;
    let raw_len = n * usize::from(h.samples) * usize::from(h.bit_depth / 8);
    let mut bounds = ::tiff::decoder::Limits::default();
    bounds.decoding_buffer_size = raw_len;
    bounds.ifd_value_size = 4 * 1024 * 1024;
    bounds.intermediate_buffer_size = (n * 8).max(1024 * 1024);
    decoder = decoder.with_limits(bounds);
    if decoder.dimensions().map_err(|_| Error::Codec)? != (h.width, h.height) {
        return Err(Error::Invalid);
    }
    let color = decoder.colortype().map_err(|_| Error::Codec)?;
    // The pinned decoder represents admitted BlackIsZero + straight alpha as
    // Multiband(2), rather than GrayA. The borrowed header parser has already
    // rejected every other multiband interpretation and associated alpha.
    let supported = matches!(
        color,
        ::tiff::ColorType::Gray(_)
            | ::tiff::ColorType::GrayA(_)
            | ::tiff::ColorType::RGB(_)
            | ::tiff::ColorType::RGBA(_)
    ) || matches!(color, ::tiff::ColorType::Multiband { num_samples: 2, .. })
        && h.samples == 2
        && h.alpha;
    if color.bit_depth() != h.bit_depth || color.num_samples() != u16::from(h.samples) || !supported
    {
        return Err(Error::Unsupported);
    }
    let mut decoded = ::tiff::decoder::DecodingResult::U8(Vec::new());
    let result = decoder.read_image_to_buffer(&mut decoded);
    check(cancel)?;
    let layout = result.map_err(|_| Error::Codec)?;
    if layout.complete_len != raw_len
        || layout.planes != if h.planar { usize::from(h.samples) } else { 1 }
    {
        return Err(Error::Invalid);
    }
    // read_image_to_buffer may legally return only the first plane under a
    // library limit. Exact lengths below prohibit accepting that partial image.
    fn expand<T: Default + Clone + Copy>(
        raw: &[T],
        h: &Header,
        max: T,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Vec<T>> {
        let n = h.width as usize * h.height as usize;
        let samples = usize::from(h.samples);
        if raw.len() != n * samples {
            return Err(Error::Invalid);
        }
        let mut out = alloc(n * 4)?;
        for i in 0..n {
            if i % 4096 == 0 {
                check(cancel)?;
            }
            let value = |c: usize| raw[if h.planar { c * n + i } else { i * samples + c }];
            let gray = samples < 3;
            let (r, g, b) = if gray {
                let v = value(0);
                (v, v, v)
            } else {
                (value(0), value(1), value(2))
            };
            let a = if h.alpha { value(samples - 1) } else { max };
            out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, b, a]);
        }
        Ok(out)
    }
    match decoded {
        ::tiff::decoder::DecodingResult::U8(raw) if h.bit_depth == 8 => {
            Ok(Pixels::Rgba8(expand(&raw, h, 255, cancel)?))
        }
        ::tiff::decoder::DecodingResult::U16(raw) if h.bit_depth == 16 => {
            Ok(Pixels::Rgba16(expand(&raw, h, 65535, cancel)?))
        }
        _ => Err(Error::Depth),
    }
}

// Uncompressed classic TIFF, written in bounded rows. ICC bytes and 8/16-bit
// channel values survive exactly. The original encoded asset is never replaced.
pub(crate) fn encode(
    image: &DecodedImage,
    out: &mut crate::io::Output<'_>,
    provenance: Option<&str>,
) -> Result<()> {
    struct Entry {
        tag: u16,
        kind: u16,
        count: u32,
        data: Vec<u8>,
    }
    let gray = image
        .icc
        .as_ref()
        .is_some_and(|p| p.get(16..20) == Some(b"GRAY"));
    let samples = if gray { 2_u32 } else { 4 };
    let depth = match &image.pixels {
        Pixels::Rgba8(_) => 8_u16,
        Pixels::Rgba16(_) => 16,
    };
    let row = u64::from(image.width) * u64::from(samples) * u64::from(depth / 8);
    let strips = image.height.div_ceil(16);
    let base_entries =
        13_usize + usize::from(image.icc.is_some()) + usize::from(provenance.is_some());
    let fixed = 8 + 2 + base_entries * 12 + 4;
    let bits = (0..samples)
        .flat_map(|_| depth.to_le_bytes())
        .collect::<Vec<_>>();
    let mut entries = vec![
        Entry {
            tag: 256,
            kind: 4,
            count: 1,
            data: image.width.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 257,
            kind: 4,
            count: 1,
            data: image.height.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 258,
            kind: 3,
            count: samples,
            data: bits,
        },
        Entry {
            tag: 259,
            kind: 3,
            count: 1,
            data: 1_u16.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 262,
            kind: 3,
            count: 1,
            data: (if gray { 1_u16 } else { 2 }).to_le_bytes().to_vec(),
        },
        Entry {
            tag: 273,
            kind: 4,
            count: strips,
            data: alloc(strips as usize * 4)?,
        },
        Entry {
            tag: 274,
            kind: 3,
            count: 1,
            data: 1_u16.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 277,
            kind: 3,
            count: 1,
            data: (samples as u16).to_le_bytes().to_vec(),
        },
        Entry {
            tag: 278,
            kind: 4,
            count: 1,
            data: 16_u32.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 279,
            kind: 4,
            count: strips,
            data: alloc(strips as usize * 4)?,
        },
        Entry {
            tag: 284,
            kind: 3,
            count: 1,
            data: 1_u16.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 338,
            kind: 3,
            count: 1,
            data: 2_u16.to_le_bytes().to_vec(),
        },
        Entry {
            tag: 339,
            kind: 3,
            count: samples,
            data: (0..samples).flat_map(|_| 1_u16.to_le_bytes()).collect(),
        },
    ];
    if let Some(profile) = &image.icc {
        entries.push(Entry {
            tag: 34675,
            kind: 7,
            count: u32::try_from(profile.len()).map_err(|_| Error::Limit)?,
            data: profile.clone(),
        });
    }
    if let Some(provenance) = provenance {
        let mut description = alloc(provenance.len() + 1)?;
        description[..provenance.len()].copy_from_slice(provenance.as_bytes());
        entries.push(Entry {
            tag: 270,
            kind: 2,
            count: u32::try_from(description.len()).map_err(|_| Error::Limit)?,
            data: description,
        });
    }
    let mut data_start = fixed;
    for e in &entries {
        if e.data.len() > 4 {
            data_start = data_start
                .checked_add(e.data.len())
                .and_then(|n| n.checked_add(n % 2))
                .ok_or(Error::Limit)?;
        }
    }
    let total = (data_start as u64)
        .checked_add(
            row.checked_mul(u64::from(image.height))
                .ok_or(Error::Limit)?,
        )
        .ok_or(Error::Limit)?;
    if total > u64::from(u32::MAX) {
        return Err(Error::Limit);
    }
    let mut cursor = data_start as u64;
    for i in 0..strips {
        let len = row * u64::from(16.min(image.height - i * 16));
        let at = i as usize * 4;
        entries[5].data[at..at + 4].copy_from_slice(&(cursor as u32).to_le_bytes());
        entries[9].data[at..at + 4].copy_from_slice(&(len as u32).to_le_bytes());
        cursor += len;
    }
    // Strip entries above use their fixed constructor positions; sort only
    // after final offsets/counts are known, as required for the TIFF IFD.
    entries.sort_unstable_by_key(|entry| entry.tag);
    let mut metadata = alloc(data_start)?;
    metadata[..4].copy_from_slice(b"II*\0");
    metadata[4..8].copy_from_slice(&8_u32.to_le_bytes());
    metadata[8..10].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    let mut extra = fixed;
    for (i, e) in entries.iter().enumerate() {
        let at = 10 + i * 12;
        metadata[at..at + 2].copy_from_slice(&e.tag.to_le_bytes());
        metadata[at + 2..at + 4].copy_from_slice(&e.kind.to_le_bytes());
        metadata[at + 4..at + 8].copy_from_slice(&e.count.to_le_bytes());
        if e.data.len() <= 4 {
            metadata[at + 8..at + 8 + e.data.len()].copy_from_slice(&e.data);
        } else {
            metadata[at + 8..at + 12].copy_from_slice(&(extra as u32).to_le_bytes());
            metadata[extra..extra + e.data.len()].copy_from_slice(&e.data);
            extra += e.data.len();
            extra += extra % 2;
        }
    }
    out.bytes(&metadata)?;
    drop(metadata);
    drop(entries);
    let mut row_bytes = alloc(row as usize)?;
    match &image.pixels {
        Pixels::Rgba8(p) => {
            for src in p.chunks_exact(image.width as usize * 4) {
                for (a, b) in src
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(row_bytes.chunks_exact_mut(samples as usize))
                {
                    if gray {
                        b.copy_from_slice(&[a[0], a[3]]);
                    } else {
                        b.copy_from_slice(a);
                    }
                }
                out.bytes(&row_bytes)?;
            }
        }
        Pixels::Rgba16(p) => {
            for src in p.chunks_exact(image.width as usize * 4) {
                for (a, b) in src
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(row_bytes.chunks_exact_mut(samples as usize * 2))
                {
                    if gray {
                        b[..2].copy_from_slice(&a[0].to_le_bytes());
                        b[2..4].copy_from_slice(&a[3].to_le_bytes());
                    } else {
                        for (v, pair) in a.iter().zip(b.as_chunks_mut::<2>().0.iter_mut()) {
                            pair.copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
                out.bytes(&row_bytes)?;
            }
        }
    }
    Ok(())
}

use crate::RasterError;
fn xmp(json: &str) -> Vec<u8> {
    let escaped = json
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:vwb=\"https://visualworkbench.app/ns/export/1.0/\"><vwb:Metadata>{escaped}</vwb:Metadata></rdf:Description></rdf:RDF></x:xmpmeta>").into_bytes()
}
pub(crate) fn jpeg_xmp(mut jpeg: Vec<u8>, json: &str) -> Result<Vec<u8>, RasterError> {
    if !jpeg.starts_with(&[0xff, 0xd8]) {
        return Err(RasterError::Codec);
    }
    let mut data = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    data.extend(xmp(json));
    let len = u16::try_from(data.len() + 2).map_err(|_| RasterError::Metadata)?;
    let mut app = vec![0xff, 0xe1];
    app.extend(len.to_be_bytes());
    app.extend(data);
    jpeg.splice(2..2, app);
    Ok(jpeg)
}
fn chunk(output: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) -> Result<(), RasterError> {
    output.extend(tag);
    output.extend(
        u32::try_from(data.len())
            .map_err(|_| RasterError::Metadata)?
            .to_le_bytes(),
    );
    output.extend(data);
    if data.len() % 2 == 1 {
        output.push(0);
    }
    Ok(())
}
pub(crate) fn webp_xmp(
    webp: Vec<u8>,
    w: u32,
    h: u32,
    alpha: bool,
    icc: Option<&[u8]>,
    json: &str,
) -> Result<Vec<u8>, RasterError> {
    if webp.len() < 12 || &webp[..4] != b"RIFF" || &webp[8..12] != b"WEBP" {
        return Err(RasterError::Codec);
    }
    let mut out = b"RIFF\0\0\0\0WEBP".to_vec();
    let mut vp8x = [0u8; 10];
    vp8x[0] = 4 | if alpha { 16 } else { 0 } | if icc.is_some() { 32 } else { 0 };
    vp8x[4..7].copy_from_slice(&(w - 1).to_le_bytes()[..3]);
    vp8x[7..10].copy_from_slice(&(h - 1).to_le_bytes()[..3]);
    chunk(&mut out, b"VP8X", &vp8x)?;
    if let Some(profile) = icc {
        chunk(&mut out, b"ICCP", profile)?;
    }
    let mut at = 12;
    while at < webp.len() {
        let head = webp.get(at..at + 8).ok_or(RasterError::Codec)?;
        let size =
            u32::from_le_bytes(head[4..8].try_into().map_err(|_| RasterError::Codec)?) as usize;
        let end = at
            .checked_add(8)
            .and_then(|v| v.checked_add(size))
            .ok_or(RasterError::Codec)?;
        let data = webp.get(at + 8..end).ok_or(RasterError::Codec)?;
        if matches!(&head[..4], b"VP8 " | b"VP8L" | b"ALPH") {
            chunk(
                &mut out,
                head[..4].try_into().map_err(|_| RasterError::Codec)?,
                data,
            )?;
        }
        at = end + size % 2;
    }
    chunk(&mut out, b"XMP ", &xmp(json))?;
    let size = u32::try_from(out.len() - 8).map_err(|_| RasterError::Metadata)?;
    out[4..8].copy_from_slice(&size.to_le_bytes());
    Ok(out)
}

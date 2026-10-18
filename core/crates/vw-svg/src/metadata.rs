//! Reuse image-export XMP vocabulary and serialized JSON, not a sidecar.
use crate::{Limits, Outline, SvgDocument, SvgError, SvgResult};
use usvg::roxmltree::{Document, ParsingOptions};

pub const MAX_EXPORT_METADATA_BYTES: usize = 16 * 1024;
const EXPORT_NS: &str = "https://visualworkbench.app/ns/export/1.0/";
fn valid(metadata: &str) -> SvgResult<()> {
    if metadata.len() > MAX_EXPORT_METADATA_BYTES {
        return Err(SvgError::Limit);
    }
    if metadata.is_empty() || !metadata.is_ascii() || metadata.as_bytes().contains(&0) {
        return Err(SvgError::Malformed);
    }
    // XML1.0 does not admit most C0 controls, even as character references.
    if metadata
        .bytes()
        .any(|v| v < 32 && !matches!(v, b'\t' | b'\n' | b'\r'))
    {
        return Err(SvgError::Malformed);
    }
    Ok(())
}
impl SvgDocument {
    /// Caller supplies validated revision/source/capture JSON. Static source
    /// metadata is discarded by the trusted tree serializer; append one fresh
    /// namespaced metadata node, then independently parse the final XML bytes.
    pub fn export_svg_with_metadata(
        &self,
        overlays: &[Outline],
        metadata: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> SvgResult<Vec<u8>> {
        valid(metadata)?;
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        let escaped = metadata
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\r', "&#13;");
        let block = format!(
            "<metadata><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:vwb=\"{EXPORT_NS}\"><vwb:Metadata>{escaped}</vwb:Metadata></rdf:Description></rdf:RDF></x:xmpmeta></metadata>"
        );
        let mut bytes = self.export_svg(overlays, cancelled)?;
        let end = bytes.len().checked_sub(6).ok_or(SvgError::Malformed)?;
        if bytes.get(end..) != Some(b"</svg>") {
            return Err(SvgError::Malformed);
        }
        let length = bytes
            .len()
            .checked_add(block.len())
            .ok_or(SvgError::Limit)?;
        if length > self.limits().output_bytes.min(self.limits().input_bytes) {
            return Err(SvgError::Limit);
        }
        bytes
            .try_reserve_exact(block.len())
            .map_err(|_| SvgError::Limit)?;
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        bytes.truncate(end);
        bytes.extend_from_slice(block.as_bytes());
        bytes.extend_from_slice(b"</svg>");
        // Retained source/tree and independent XML readback coexist.
        self.admit_metadata_readback(bytes.len())?;
        verify_svg_copy_metadata(&bytes, metadata, self.limits(), cancelled)?;
        Ok(bytes)
    }
}

/// Exact namespace/tree readback, never substring matching. Wrong namespace,
/// duplicate metadata, nested active markup and a matching comment all refuse.
pub fn verify_svg_copy_metadata(
    bytes: &[u8],
    expected: &str,
    limits: Limits,
    cancelled: &dyn Fn() -> bool,
) -> SvgResult<()> {
    valid(expected)?;
    limits.validate()?;
    if cancelled() {
        return Err(SvgError::Cancelled);
    }
    if bytes.len() > limits.output_bytes.min(limits.input_bytes) {
        return Err(SvgError::Limit);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| SvgError::Malformed)?;
    // Match preflight's bounds on parser reservations before roxmltree allocates.
    if bytes.iter().filter(|v| **v == b'<').count() > limits.nodes as usize * 2
        || bytes.iter().filter(|v| **v == b'=').count() > limits.nodes as usize * 32
    {
        return Err(SvgError::Limit);
    }
    limits.parse_memory(text.len(), text.len() as u64)?;
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err(SvgError::DtdDenied);
    }
    let doc = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: limits.nodes,
            entity_resolver: None,
        },
    )
    .map_err(|_| SvgError::Malformed)?;
    let root = doc.root_element();
    if !root.has_tag_name(("http://www.w3.org/2000/svg", "svg")) {
        return Err(SvgError::Malformed);
    }
    let mut values = root
        .descendants()
        .filter(|n| n.has_tag_name((EXPORT_NS, "Metadata")));
    let value = values.next().ok_or(SvgError::Malformed)?;
    if values.next().is_some()
        || value.children().any(|n| !n.is_text())
        || value.text() != Some(expected)
    {
        return Err(SvgError::Malformed);
    }
    let description = value.parent_element().ok_or(SvgError::Malformed)?;
    let rdf = description.parent_element().ok_or(SvgError::Malformed)?;
    let xmp = rdf.parent_element().ok_or(SvgError::Malformed)?;
    let metadata = xmp.parent_element().ok_or(SvgError::Malformed)?;
    if !description.has_tag_name(("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "Description"))
        || !rdf.has_tag_name(("http://www.w3.org/1999/02/22-rdf-syntax-ns#", "RDF"))
        || !xmp.has_tag_name(("adobe:ns:meta/", "xmpmeta"))
        || !metadata.has_tag_name(("http://www.w3.org/2000/svg", "metadata"))
        || metadata.parent_element() != Some(root)
    {
        return Err(SvgError::Malformed);
    }
    if cancelled() {
        return Err(SvgError::Cancelled);
    }
    Ok(())
}

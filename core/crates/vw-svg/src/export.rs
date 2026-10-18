//! Export only the simplified static tree and validated owned vector overlays.
use crate::{SvgDocument, SvgError, SvgResult};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    /// A single compound NONZERO fill; highlighter contours are painted once.
    pub contours: Vec<Vec<Point>>,
    /// [a,b,c,d,e,f], applied in D space. Reflection preserves contour winding.
    pub transform: [f64; 6],
    pub rgba: [u8; 4],
}

/// Checks before every append, including formatter fragments and closing tags.
/// The trusted-tree serializer has its separate process-bound reservation; an
/// admitted overlay cannot grow this buffer past the explicit output ceiling.
struct CappedXml<'a> {
    xml: &'a mut String,
    cap: usize,
}

impl std::fmt::Write for CappedXml<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        let next = self
            .xml
            .len()
            .checked_add(value.len())
            .ok_or(std::fmt::Error)?;
        if next > self.cap {
            return Err(std::fmt::Error);
        }
        self.xml
            .try_reserve_exact(value.len())
            .map_err(|_| std::fmt::Error)?;
        self.xml.push_str(value);
        Ok(())
    }
}

impl SvgDocument {
    /// Re-emits text as paths using bundled fonts. Active input XML is never
    /// embedded. The returned XML contains no links or external resource path.
    pub fn export_svg(
        &self,
        overlays: &[Outline],
        cancelled: &dyn Fn() -> bool,
    ) -> SvgResult<Vec<u8>> {
        if cancelled() {
            return Err(SvgError::Cancelled);
        }
        if overlays.len() > 4096 {
            return Err(SvgError::Limit);
        }
        let mut total = 0_usize;
        for item in overlays {
            if item.contours.is_empty()
                || item.contours.len() > 65_536
                || item
                    .transform
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 1e9)
            {
                return Err(SvgError::InvalidOverlay);
            }
            let [a, b, c, d, _, _] = item.transform;
            if (a * d - b * c).abs() < 1e-12 {
                return Err(SvgError::InvalidOverlay);
            }
            for contour in &item.contours {
                if cancelled() {
                    return Err(SvgError::Cancelled);
                }
                total = total.checked_add(contour.len()).ok_or(SvgError::Limit)?;
                if contour.len() < 3 || total > 250_000 {
                    return Err(SvgError::Limit);
                }
                if contour.iter().any(|p| {
                    !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e9 || p.y.abs() > 1e9
                }) {
                    return Err(SvgError::InvalidOverlay);
                }
            }
        }
        // Finite f64 `.17e` occupies at most 25 ASCII bytes, including a sign
        // and the three-digit subnormal exponent. Two coordinates, command,
        // separators and contour closure fit 64 bytes per admitted point.
        let reserve = total
            .checked_mul(64)
            .and_then(|v| v.checked_add(overlays.len() * 512))
            .and_then(|v| v.checked_add(8))
            .ok_or(SvgError::Limit)?;
        if reserve > self.limits().output_bytes {
            return Err(SvgError::Limit);
        }
        // usvg has no bounded writer API. The admitted process memory cap and
        // source/expanded-geometry bounds cover this one normalized serialization.
        let mut xml = self.tree.to_string(&usvg::WriteOptions {
            preserve_text: false,
            coordinates_precision: 9,
            transforms_precision: 9,
            ..usvg::WriteOptions::default()
        });
        let cap = self.limits().output_bytes.min(self.limits().input_bytes);
        if xml.len().checked_add(reserve).is_none_or(|v| v > cap) {
            return Err(SvgError::Limit);
        }
        if let Some(end) = xml.rfind("</svg>") {
            xml.truncate(end);
        } else {
            // The trusted usvg writer emits an empty/fully-denied tree as one
            // self-closing root. Expand that root before appending our paths.
            let end = xml.trim_end().len();
            if !xml[..end].ends_with("/>") || !xml.starts_with("<svg") {
                return Err(SvgError::Malformed);
            }
            xml.truncate(end - 2);
            CappedXml { xml: &mut xml, cap }
                .write_char('>')
                .map_err(|_| SvgError::Limit)?;
        }
        xml.try_reserve_exact(reserve)
            .map_err(|_| SvgError::Limit)?;
        let mut sink = CappedXml { xml: &mut xml, cap };
        for item in overlays {
            if cancelled() {
                return Err(SvgError::Cancelled);
            }
            let [a, b, c, d, e, f] = item.transform;
            let [r, g, b_color, alpha] = item.rgba;
            write!(sink, "<path fill=\"#{r:02x}{g:02x}{b_color:02x}\" fill-opacity=\"{:.17e}\" fill-rule=\"nonzero\" transform=\"matrix({a:.17e} {b:.17e} {c:.17e} {d:.17e} {e:.17e} {f:.17e})\" d=\"", f64::from(alpha) / 255.0).map_err(|_| SvgError::Limit)?;
            for contour in &item.contours {
                if cancelled() {
                    return Err(SvgError::Cancelled);
                }
                for (index, point) in contour.iter().enumerate() {
                    let command = if index == 0 { 'M' } else { 'L' };
                    write!(sink, "{command}{:.17e} {:.17e} ", point.x, point.y)
                        .map_err(|_| SvgError::Limit)?;
                }
                sink.write_str("Z ").map_err(|_| SvgError::Limit)?;
            }
            sink.write_str("\"/>").map_err(|_| SvgError::Limit)?;
        }
        sink.write_str("</svg>").map_err(|_| SvgError::Limit)?;
        Ok(xml.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_sink_refuses_before_append_including_formatted_fragments() {
        let mut xml = String::from("abc");
        let mut sink = CappedXml {
            xml: &mut xml,
            cap: 6,
        };
        assert!(sink.write_str("defg").is_err());
        assert_eq!(sink.xml.as_str(), "abc");
        assert!(write!(sink, "{:.17e}", f64::from_bits(1)).is_err());
        assert!(sink.xml.len() <= 6);
    }
}

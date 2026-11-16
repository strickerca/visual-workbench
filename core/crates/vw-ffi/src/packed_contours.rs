//! The compact, lossless form in which `document_snapshot_packed` and
//! `document_snapshot_changes_packed` send a stroke's outline instead of
//! `Contours`. The layout is described on `PackedRenderItem::contours`; it
//! carries exactly the vertices and contour ends `Contours` would carry.
use crate::{CoreError, Result};

/// Written first in every non-empty buffer.
pub(crate) const VERSION: u64 = 1;

fn zigzag(value: i64) -> u64 {
    ((value << 1) ^ (value >> 63)) as u64
}
fn varint_len(value: u64) -> usize {
    // One byte per started group of seven bits; zero takes one byte.
    (64 - (value | 1).leading_zeros() as usize).div_ceil(7)
}
fn put(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
/// Every number of the packed form, in order.
fn walk(polygons: &[vw_ink::Polygon], vertices: usize, mut emit: impl FnMut(u64)) {
    emit(VERSION);
    emit(polygons.len() as u64);
    emit(vertices as u64);
    let (mut x, mut y) = (0_i64, 0_i64);
    for polygon in polygons {
        emit(polygon.points.len() as u64);
        for point in &polygon.points {
            emit(zigzag(point.x.wrapping_sub(x)));
            emit(zigzag(point.y.wrapping_sub(y)));
            (x, y) = (point.x, point.y);
        }
    }
}
/// The packed outline of `polygons`, allocated at exactly its length. Empty
/// when there is no contour. Refuses what `gesture::contours` refuses.
pub(crate) fn pack(polygons: &[vw_ink::Polygon]) -> Result<Vec<u8>> {
    let vertices = polygons.iter().try_fold(0usize, |n, polygon| {
        n.checked_add(polygon.points.len())
            .ok_or(CoreError::Invalid)
    })?;
    if vertices > vw_ink::MAX_VERTICES {
        return Err(CoreError::Invalid);
    }
    let mut out = Vec::new();
    if polygons.is_empty() {
        return Ok(out);
    }
    // At most 4,000,000 vertices of 20 bytes plus one header per contour:
    // far below usize::MAX, so the sum cannot overflow.
    let mut length = 0usize;
    walk(polygons, vertices, |value| length += varint_len(value));
    out.try_reserve_exact(length)
        .map_err(|_| CoreError::Worker)?;
    walk(polygons, vertices, |value| put(&mut out, value));
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::Contours;
    use vw_ink::{Polygon, QPoint};

    /// An independent reader of the documented layout.
    fn unpack(bytes: &[u8]) -> Contours {
        let mut at = 0;
        let mut next = || {
            let mut value = 0_u64;
            let mut shift = 0;
            loop {
                let byte = bytes[at];
                at += 1;
                value |= u64::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    return value;
                }
                shift += 7;
                assert!(shift < 64, "varint longer than ten bytes");
            }
        };
        let signed = |value: u64| ((value >> 1) as i64) ^ -((value & 1) as i64);
        let mut result = Contours::default();
        if bytes.is_empty() {
            return result;
        }
        assert_eq!(next(), VERSION);
        let contours = next();
        let vertices = next();
        let (mut x, mut y) = (0_i64, 0_i64);
        for _ in 0..contours {
            for _ in 0..next() {
                x = x.wrapping_add(signed(next()));
                y = y.wrapping_add(signed(next()));
                result.x.push(x);
                result.y.push(y);
            }
            result.ends.push(result.x.len() as u32);
        }
        assert_eq!(result.x.len() as u64, vertices);
        assert_eq!(at, bytes.len(), "trailing bytes");
        result
    }
    fn polygon(points: &[(i64, i64)]) -> Polygon {
        Polygon {
            points: points.iter().map(|&(x, y)| QPoint { x, y }).collect(),
        }
    }

    #[test]
    fn the_layout_is_pinned() {
        // Two triangles; the second starts 256 units (one pixel) to the right.
        let polygons = [
            polygon(&[(0, 0), (256, 0), (0, 256)]),
            polygon(&[(512, -1), (768, 0), (512, 256)]),
        ];
        assert_eq!(
            pack(&polygons).unwrap(),
            [
                1, 2, 6, // version, contours, vertices
                3, 0, 0, 128, 4, 0, 255, 3, 128, 4, // (0,0) (+256,0) (-256,+256)
                3, 128, 8, 129, 4, 128, 4, 2, 255, 3, 128,
                4, // (+512,-257) (+256,+1) (-256,+256)
            ]
        );
        assert!(pack(&[]).unwrap().is_empty());
    }

    #[test]
    fn every_value_comes_back_exactly_as_the_lists_carry_it() {
        let extremes = [
            i64::MIN,
            i64::MIN + 1,
            -1 << 40,
            -257,
            -1,
            0,
            1,
            63,
            64,
            8191,
            8192,
            1 << 40,
            i64::MAX - 1,
            i64::MAX,
        ];
        let mut polygons = Vec::new();
        for (n, &a) in extremes.iter().enumerate() {
            let b = extremes[(n * 5 + 3) % extremes.len()];
            polygons.push(polygon(&[(a, b), (b, a), (a.wrapping_add(7), b), (0, a)]));
        }
        // Coordinates a real stroke can produce: up to (1e9 + 4096) * 256.
        let limit =
            ((vw_ink::MAX_COORDINATE + vw_ink::MAX_BASE_WIDTH) * vw_ink::QUANTIZATION) as i64;
        polygons.push(polygon(&[(-limit, limit), (limit, -limit), (limit, limit)]));
        let packed = pack(&polygons).unwrap();
        assert_eq!(packed.capacity(), packed.len(), "allocated at its length");
        assert_eq!(
            unpack(&packed),
            crate::gesture::contours(&polygons).unwrap()
        );
    }

    #[test]
    fn a_real_stroke_outline_is_identical_and_about_a_quarter_of_its_lists() {
        let samples = 100;
        let stroke = vw_proto::v1::Stroke {
            brush: Some(vw_proto::v1::Brush {
                family: "pen".into(),
                algorithm_version: vw_ink::ALGORITHM_VERSION,
                base_width: 6.0,
                pressure_curve: vec![0.0, 0.0, 0.25, 0.25, 0.75, 0.75, 1.0, 1.0],
                stabilization: 0.15,
            }),
            x: (0..samples).map(|i| 100.0 + f64::from(i) * 1.7).collect(),
            y: (0..samples)
                .map(|i| 200.0 + (f64::from(i) * 0.35).sin() * 9.0)
                .collect(),
            t_ms: (0..samples).map(|i| i as u32 * 4).collect(),
            pressure: (0..samples)
                .map(|i| 0.45 + 0.4 * (f32::from(i as u8) * 0.11).sin().abs())
                .collect(),
            ..Default::default()
        };
        let geometry = vw_ink::geometry_from_stroke(&stroke).unwrap();
        let lists = crate::gesture::contours(geometry.polygons()).unwrap();
        let packed = pack(geometry.polygons()).unwrap();
        assert_eq!(unpack(&packed), lists);
        assert!(lists.x.len() > 10 * samples as usize);
        // Two i64 per vertex against a few bytes per vertex.
        assert!(
            packed.len() * 3 < lists.x.len() * 16,
            "{} bytes",
            packed.len()
        );
    }
}

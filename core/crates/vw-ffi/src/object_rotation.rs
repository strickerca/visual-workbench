use crate::{CoreError, Point, Result, Transform};
use vw_geom::{Affine, Point as DPoint};

/// Bounded synchronous document-space math. It creates no handle or worker,
/// reads no project and cannot commit. Apps submit the returned transforms via
/// the existing full-revision-CAS gesture transaction exactly once on pen-up.
#[uniffi::export]
pub fn rotate_object_transforms(
    originals: Vec<Transform>,
    pivot: Point,
    start: Point,
    current: Point,
    snap_15: bool,
) -> Result<Vec<Transform>> {
    if originals.is_empty() || originals.len() > 256 {
        return Err(CoreError::Invalid);
    }
    let outer = vw_geom::object_rotation(
        DPoint::new(pivot.x, pivot.y)?,
        DPoint::new(start.x, start.y)?,
        DPoint::new(current.x, current.y)?,
        snap_15,
    )?;
    let mut result = Vec::with_capacity(originals.len());
    for original in originals {
        crate::editor::affine(original)?;
        let affine = Affine::new(
            original.a, original.b, original.c, original.d, original.e, original.f,
        )?;
        affine.inverse()?;
        if outer == Affine::IDENTITY {
            result.push(original);
            continue;
        }
        let [a, b, c, d, e, f] = affine.then(outer)?.coefficients();
        let transformed = Transform { a, b, c, d, e, f };
        crate::editor::affine(transformed)?;
        result.push(transformed);
    }
    Ok(result)
}

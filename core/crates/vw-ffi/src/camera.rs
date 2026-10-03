use crate::*;

fn matrix(camera: Camera) -> Result<vw_geom::Affine> {
    Ok(vw_geom::Camera::new(
        vw_geom::Point::new(camera.center.x, camera.center.y)?,
        camera.scale,
        camera.rotation,
    )?
    .view_matrix(vw_geom::Size::new(
        camera.viewport_width,
        camera.viewport_height,
    )?)?)
}
#[uniffi::export]
pub fn camera_matrix(camera: Camera, inverse: bool) -> Result<Transform> {
    let m = matrix(camera)?;
    let m = if inverse { m.inverse()? } else { m };
    let [a, b, c, d, e, f] = m.coefficients();
    Ok(Transform { a, b, c, d, e, f })
}
/// One bounded mapping per input batch. No I/O, shared state, or f32 conversion.
#[uniffi::export]
pub fn camera_map_points(camera: Camera, inverse: bool, points: Vec<Point>) -> Result<Vec<Point>> {
    if points.len() > MAX_BATCH_SAMPLES {
        return Err(CoreError::Invalid);
    }
    let m = matrix(camera)?;
    let m = if inverse { m.inverse()? } else { m };
    points
        .into_iter()
        .map(|p| {
            let p = m.map(vw_geom::Point::new(p.x, p.y)?)?;
            Ok(Point { x: p.x(), y: p.y() })
        })
        .collect()
}

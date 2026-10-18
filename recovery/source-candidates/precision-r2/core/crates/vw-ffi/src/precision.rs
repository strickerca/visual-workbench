//! Constant-sized precision geometry. Inspection never calls this input route.
use crate::{Camera, CoreError, Point, Result, Transform};
use std::sync::{Arc, Mutex};
use vw_geom::{Affine, LensTransform};

#[derive(Clone, Debug, uniffi::Record)]
pub struct PrecisionMapping {
    pub base: Transform,
    pub lens: Transform,
    pub open: bool,
    pub latched: bool,
    pub lens_input: bool,
}
struct State {
    base: Affine,
    lens: LensTransform,
    // A down outside the inset stays in base space even after crossing its edge.
    input: Option<bool>,
}
#[derive(uniffi::Object)]
pub struct PrecisionLens {
    state: Mutex<Option<State>>,
}
fn point(p: Point) -> Result<vw_geom::Point> { Ok(vw_geom::Point::new(p.x, p.y)?) }
fn transform(value: Affine) -> Transform {
    let [a,b,c,d,e,f] = value.coefficients(); Transform {a,b,c,d,e,f}
}
fn camera_base(camera: Camera) -> Result<Affine> {
    Ok(vw_geom::Camera::new(point(camera.center)?, camera.scale, camera.rotation)?
        .view_matrix(vw_geom::Size::new(camera.viewport_width, camera.viewport_height)?)?)
}
/// Inspection/render-only matrix; it never changes an input handle.
#[uniffi::export]
pub fn precision_view_transform(camera: Camera, focus: Point, center: Point, magnification: u8) -> Result<Transform> {
    let mut lens = LensTransform::new(camera_base(camera)?)?;
    lens.open_magnified(point(focus)?, point(center)?, magnification)?;
    Ok(transform(lens.document_to_input()))
}
#[uniffi::export]
pub fn create_precision_lens(camera: Camera) -> Result<Arc<PrecisionLens>> {
    let base = camera_base(camera)?;
    Ok(Arc::new(PrecisionLens { state: Mutex::new(Some(State {base, lens: LensTransform::new(base)?, input: None})) }))
}
impl PrecisionLens {
    fn with<T>(&self, action: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        let mut state = self.state.try_lock().map_err(|_| CoreError::Backpressure)?;
        action(state.as_mut().ok_or(CoreError::Closed)?)
    }
}
#[uniffi::export]
impl PrecisionLens {
    /// Bounded synchronous math only. The canonical lens queues changes while latched.
    pub fn configure(&self, focus: Point, center: Point, magnification: u8) -> Result<()> {
        self.with(|s| { s.lens.open_magnified(point(focus)?, point(center)?, magnification)?; Ok(()) })
    }
    pub fn close_lens(&self) -> Result<()> { self.with(|s| {s.lens.close(); Ok(())}) }
    pub fn down(&self, inside: bool) -> Result<()> {
        self.with(|s| {
            s.lens.latch()?;
            s.input = Some(inside && s.lens.is_open());
            Ok(())
        })
    }
    pub fn up(&self) -> Result<()> {
        self.with(|s| {s.lens.unlatch()?; s.input = None; Ok(())})
    }
    /// Turn accepted/predicted input into existing base-camera pixels. All samples
    /// use the pen-down choice and inverse; an inspection-only path returns the
    /// original f64 values unchanged, including signed zero.
    pub fn map_to_base(&self, points: Vec<Point>, inside: bool) -> Result<Vec<Point>> {
        if points.len() > 512 { return Err(CoreError::Invalid); }
        self.with(|s| {
            let use_lens = s.input.unwrap_or(inside && s.lens.is_open());
            if !use_lens {
                for p in &points { vw_geom::Point::new(p.x,p.y)?; }
                return Ok(points);
            }
            points.into_iter().map(|p| {
                let mapped = s.base.map(s.lens.input_to_document(point(p)?)?)?;
                Ok(Point{x:mapped.x(),y:mapped.y()})
            }).collect()
        })
    }
    pub fn mapping(&self) -> Result<PrecisionMapping> {
        self.with(|s| Ok(PrecisionMapping {base:transform(s.base),lens:transform(s.lens.document_to_input()),
            open:s.lens.is_open(),latched:s.lens.is_latched(),lens_input:s.input == Some(true)}))
    }
    /// Render-only footprint; independent of a stroke begun outside the inset.
    pub fn view_to_base(&self, points: Vec<Point>) -> Result<Vec<Point>> {
        if points.len() > 512 { return Err(CoreError::Invalid); }
        self.with(|s| {
            let inverse = s.lens.document_to_input().inverse()?;
            points.into_iter().map(|p| {
                let mapped = s.base.map(inverse.map(point(p)?)?)?;
                Ok(Point{x:mapped.x(),y:mapped.y()})
            }).collect()
        })
    }
    /// No I/O or worker survives disposal. A later call fails closed.
    pub fn dispose(&self) -> Result<()> {
        let mut state = self.state.try_lock().map_err(|_| CoreError::Backpressure)?;
        *state = None; Ok(())
    }
}

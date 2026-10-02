use crate::{Affine, GeometryError, Point};

#[derive(Debug, Clone, Copy, PartialEq)]
struct Mapping {
    forward: Affine,
    inverse: Affine,
}

impl Mapping {
    fn new(forward: Affine) -> Result<Self, GeometryError> {
        Ok(Self {
            forward,
            inverse: forward.inverse()?,
        })
    }
}

/// Precision input space L with a transform snapshot latched at pen-down.
///
/// All stroke samples use that snapshot, including samples after a requested
/// toggle. Changes queue until pen-up/cancel (`unlatch`). Closing restores the
/// original base view coefficients exactly. Inspection loupes must not use this
/// input-changing type. The caller owns the base camera while a lens is open.
#[derive(Debug, Clone, PartialEq)]
pub struct LensTransform {
    base: Mapping,
    lens: Option<Mapping>,
    latched: Option<Mapping>,
    pending: Option<Option<Mapping>>,
}

impl LensTransform {
    /// Save an invertible base D-to-input view without reconstructing it later.
    pub fn new(base_view: Affine) -> Result<Self, GeometryError> {
        Ok(Self {
            base: Mapping::new(base_view)?,
            lens: None,
            latched: None,
            pending: None,
        })
    }
    /// Set a D-to-L transform, or queue it during a stroke; last request wins.
    /// A rejected singular transform leaves the active and queued state unchanged.
    pub fn open(&mut self, document_to_lens: Affine) -> Result<(), GeometryError> {
        let mapping = Mapping::new(document_to_lens)?;
        self.request(Some(mapping));
        Ok(())
    }
    /// Open a 2x/4x/8x lens relative to the saved base camera. The document focus
    /// maps to `lens_center` in input pixels while preserving base view rotation.
    pub fn open_magnified(
        &mut self,
        focus: Point,
        lens_center: Point,
        magnification: u8,
    ) -> Result<(), GeometryError> {
        if !matches!(magnification, 2 | 4 | 8) {
            return Err(GeometryError::InvalidMagnification);
        }
        let base_focus = self.base.forward.map(focus)?;
        let scale = f64::from(magnification);
        self.open(
            self.base
                .forward
                .then(Affine::translation(-base_focus.x(), -base_focus.y())?)?
                .then(Affine::scale(scale, scale)?)?
                .then(Affine::translation(lens_center.x(), lens_center.y())?)?,
        )
    }
    /// Restore the exact base view now, or queue that restoration until pen-up.
    pub fn close(&mut self) {
        self.request(None);
    }
    fn request(&mut self, mapping: Option<Mapping>) {
        if self.latched.is_some() {
            self.pending = Some(mapping);
        } else {
            self.lens = mapping;
        }
    }
    fn active(&self) -> Mapping {
        self.lens.unwrap_or(self.base)
    }
    /// Freeze the current forward/inverse pair at pen-down; duplicate downs fail.
    pub fn latch(&mut self) -> Result<(), GeometryError> {
        if self.latched.is_some() {
            return Err(GeometryError::AlreadyLatched);
        }
        self.latched = Some(self.active());
        Ok(())
    }
    /// End or cancel the stroke and apply the last queued lens toggle, if any.
    pub fn unlatch(&mut self) -> Result<(), GeometryError> {
        if self.latched.take().is_none() {
            return Err(GeometryError::NotLatched);
        }
        if let Some(mapping) = self.pending.take() {
            self.lens = mapping;
        }
        Ok(())
    }
    /// Map input through the pen-down inverse during a stroke, current view otherwise.
    pub fn input_to_document(&self, input: Point) -> Result<Point, GeometryError> {
        self.latched
            .unwrap_or_else(|| self.active())
            .inverse
            .map(input)
    }
    /// The exact active D-to-input matrix; queued toggles are not yet visible.
    pub fn document_to_input(&self) -> Affine {
        self.active().forward
    }
    /// Whether a pen-down snapshot is currently held.
    pub const fn is_latched(&self) -> bool {
        self.latched.is_some()
    }
    /// Whether a precision lens is currently active (including a latched lens).
    pub const fn is_open(&self) -> bool {
        self.lens.is_some()
    }
}

use crate::*;
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub memory_bytes: u64,
    pub png_bytes: u64,
    pub max_pixels: u64,
    pub max_elements: usize,
    pub max_text_bytes: usize,
    pub tree_ms: u64,
    pub capture_ms: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_bytes: 256 * 1024 * 1024,
            png_bytes: 64 * 1024 * 1024,
            max_pixels: 50_000_000,
            max_elements: 4096,
            max_text_bytes: 2 * 1024 * 1024,
            tree_ms: 300,
            capture_ms: 3000,
        }
    }
}
impl Limits {
    pub fn validate(self) -> Result<Self> {
        if self.memory_bytes == 0
            || self.memory_bytes > 256 * 1024 * 1024
            || self.png_bytes == 0
            || self.png_bytes > 64 * 1024 * 1024
            || self.max_pixels == 0
            || self.max_pixels > 50_000_000
            || self.max_elements == 0
            || self.max_elements > 4096
            || self.max_text_bytes == 0
            || self.max_text_bytes > 2 * 1024 * 1024
            || self.tree_ms == 0
            || self.tree_ms > 300
            || self.capture_ms == 0
            || self.capture_ms > 5000
        {
            Err(Error::Limit)
        } else {
            Ok(self)
        }
    }
    pub fn image(self, width: u32, height: u32) -> Result<usize> {
        self.validate()?;
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(Error::Limit)?;
        let peak = pixels
            .checked_mul(16)
            .and_then(|v| v.checked_add(16 * 1024 * 1024))
            .ok_or(Error::Limit)?;
        if width == 0
            || height == 0
            || width > 32768
            || height > 32768
            || pixels > self.max_pixels
            || peak > self.memory_bytes
        {
            return Err(Error::Limit);
        }
        usize::try_from(pixels * 4).map_err(|_| Error::Limit)
    }
}
impl Rect {
    pub fn validate(self) -> Result<Self> {
        if self.width == 0
            || self.height == 0
            || i64::from(self.x) + i64::from(self.width) > i64::from(i32::MAX)
            || i64::from(self.y) + i64::from(self.height) > i64::from(i32::MAX)
        {
            Err(Error::Invalid)
        } else {
            Ok(self)
        }
    }
    pub fn offset_inside(self, outer: Self) -> Result<(u32, u32)> {
        self.validate()?;
        outer.validate()?;
        let x = i64::from(self.x) - i64::from(outer.x);
        let y = i64::from(self.y) - i64::from(outer.y);
        if x < 0
            || y < 0
            || x + i64::from(self.width) > i64::from(outer.width)
            || y + i64::from(self.height) > i64::from(outer.height)
        {
            return Err(Error::Stale);
        }
        Ok((x as u32, y as u32))
    }
}
impl WindowTarget {
    pub fn validate(&self, owner: u32) -> Result<()> {
        if owner == 0
            || self.process_id == 0
            || self.window == 0
            || self.process_created == 0
            || self.observed_ns == 0
            || !(48..=960).contains(&self.dpi)
        {
            return Err(Error::Invalid);
        }
        if self.process_id == owner {
            return Err(Error::OwnWindow);
        }
        self.client.offset_inside(self.frame)?;
        Ok(())
    }
    pub fn unchanged(&self, current: &Self, owner: u32) -> Result<()> {
        self.validate(owner)?;
        current.validate(owner)?;
        if self.window != current.window
            || self.process_id != current.process_id
            || self.process_created != current.process_created
            || self.client != current.client
            || self.frame != current.frame
            || self.dpi != current.dpi
            || current.observed_ns < self.observed_ns
        {
            Err(Error::Stale)
        } else {
            Ok(())
        }
    }
}
impl FrameIdentity {
    pub fn validate(&self) -> Result<()> {
        vw_model::Id::try_from(self.capture_session_id.clone()).map_err(|_| Error::Invalid)?;
        self.client_rect.validate()?;
        if self.frame_id == 0
            || self.frame_id > i64::MAX as u64
            || self.geometry_revision == 0
            || self.monotonic_timestamp_ns == 0
            || self.monotonic_timestamp_ns > i64::MAX as u64
            || self.captured_at_ms < 0
            || !self.dpi_scale.is_finite()
            || self.dpi_scale <= 0.0
            || self.dpi_scale > 10.0
            || !matches!(self.platform.as_str(), "windows" | "android")
            || !matches!(self.source_kind.as_str(), "window" | "monitor")
            || self.monitor_id.len() > 256
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}
pub fn frame_delta_ms(frame_ns: u64, observed_ns: u64) -> Result<i32> {
    let difference = i128::from(observed_ns) - i128::from(frame_ns);
    i32::try_from(difference / 1_000_000).map_err(|_| Error::Limit)
}
pub fn admit_element(element: &Element, retained: &mut usize, limits: Limits) -> Result<()> {
    if element.local_id.is_empty()
        || element.local_id.len() > 256
        || element
            .parent_local_id
            .as_ref()
            .is_some_and(|v| v.is_empty() || v.len() > 256)
        || element.name.len() > 4096
        || element.role.len() > 256
        || element.text.chars().count() > 200
        || element.bounds.iter().any(|v| !v.is_finite())
        || element.bounds[2] < 0.0
        || element.bounds[3] < 0.0
    {
        return Err(Error::Limit);
    }
    let mut n =
        element.local_id.len() + element.name.len() + element.role.len() + element.text.len();
    for value in [
        &element.parent_local_id,
        &element.automation_id,
        &element.resource_id,
        &element.html_id,
    ]
    .into_iter()
    .flatten()
    {
        if value.len() > 1024 {
            return Err(Error::Limit);
        }
        n = n.checked_add(value.len()).ok_or(Error::Limit)?;
    }
    *retained = retained.checked_add(n).ok_or(Error::Limit)?;
    if *retained > limits.max_text_bytes {
        Err(Error::Limit)
    } else {
        Ok(())
    }
}

//! Bounded primitive-only JNI bridge for the disposable A/B comparison app.
//! IDs never expose pointers, and stale IDs cannot address a later allocation.
mod ffi;

use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
};
use vw_ink::{Brush, BrushFamily, Sample, StrokeBuilder};

const MAX_HANDLES: usize = 8;
const MAX_SAMPLES: usize = 2048;
#[derive(Default)]
struct Registry {
    next: i64,
    strokes: BTreeMap<i64, StrokeBuilder>,
}
static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn access<T>(fallback: T, operation: impl FnOnce(&mut Registry) -> Option<T>) -> T {
    let Ok(mut registry) = REGISTRY
        .get_or_init(|| Mutex::new(Registry::default()))
        .lock()
    else {
        return fallback;
    };
    operation(&mut registry).unwrap_or(fallback)
}
impl Registry {
    fn insert(&mut self, builder: StrokeBuilder) -> Option<i64> {
        if self.strokes.len() >= MAX_HANDLES {
            return None;
        }
        let next = self.next.checked_add(1)?;
        self.next = next;
        self.strokes.insert(next, builder);
        Some(next)
    }
}

fn begin(family: i32, width: f64, stabilization: f64) -> i64 {
    let family = match family {
        0 => BrushFamily::Pen,
        1 => BrushFamily::Marker,
        2 => BrushFamily::Highlighter,
        3 => BrushFamily::VectorEraser,
        _ => return 0,
    };
    if !stabilization.is_finite() || !(0.0..=1.0).contains(&stabilization) {
        return 0;
    }
    let Ok(mut brush) = Brush::new(family, width) else {
        return 0;
    };
    brush.stabilization = stabilization as f32;
    let Ok(builder) = StrokeBuilder::begin(brush) else {
        return 0;
    };
    access(0, |registry| registry.insert(builder))
}
fn sample(x: f64, y: f64, time: i64, pressure: f64) -> Option<Sample> {
    Sample::new(x, y, u32::try_from(time).ok()?, pressure).ok()
}
fn append(handle: i64, x: f64, y: f64, time: i64, pressure: f64) -> i32 {
    access(-1, |registry| {
        let builder = registry.strokes.get_mut(&handle)?;
        if builder.sample_count() >= MAX_SAMPLES {
            return None;
        }
        let range = builder.append(&[sample(x, y, time, pressure)?]).ok()?;
        i32::try_from(range.end).ok()
    })
}
fn preview(handle: i64, x: f64, y: f64, time: i64, pressure: f64) -> i64 {
    access(0, |registry| {
        let original = registry.strokes.get(&handle)?;
        if original.sample_count() >= MAX_SAMPLES || registry.strokes.len() >= MAX_HANDLES {
            return None;
        }
        let mut preview = original.clone();
        preview.append(&[sample(x, y, time, pressure)?]).ok()?;
        registry.insert(preview)
    })
}
fn polygons(handle: i64) -> i32 {
    access(-1, |r| {
        i32::try_from(r.strokes.get(&handle)?.geometry().polygons().len()).ok()
    })
}
fn vertices(handle: i64, polygon: i32) -> i32 {
    access(-1, |r| {
        i32::try_from(
            r.strokes
                .get(&handle)?
                .geometry()
                .polygons()
                .get(usize::try_from(polygon).ok()?)?
                .points
                .len(),
        )
        .ok()
    })
}
fn coordinate(handle: i64, polygon: i32, vertex: i32, axis: i32) -> f64 {
    access(f64::NAN, |r| {
        let point = r
            .strokes
            .get(&handle)?
            .geometry()
            .polygons()
            .get(usize::try_from(polygon).ok()?)?
            .points
            .get(usize::try_from(vertex).ok()?)?;
        match axis {
            0 => Some(point.x_px()),
            1 => Some(point.y_px()),
            _ => None,
        }
    })
}
fn finish(handle: i64) -> i32 {
    access(-1, |r| {
        r.strokes.get_mut(&handle)?.finish().ok()?;
        Some(0)
    })
}
fn hash_word(handle: i64, word: i32) -> i64 {
    access(0, |r| {
        let hash = r.strokes.get(&handle)?.geometry().hash().ok()?;
        let start = usize::try_from(word).ok()?.checked_mul(16)?;
        let end = start.checked_add(16)?;
        let word = u64::from_str_radix(hash.as_str().get(start..end)?, 16).ok()?;
        Some(i64::from_ne_bytes(word.to_ne_bytes()))
    })
}
fn release(handle: i64) -> i32 {
    access(-1, |r| r.strokes.remove(&handle).map(|_| 0))
}
fn live_handles() -> i32 {
    access(-1, |r| i32::try_from(r.strokes.len()).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn opaque_registry_bounds_stale_ids_prediction_and_finalization() {
        assert_eq!(begin(99, 12.0, 0.0), 0);
        assert_eq!(begin(0, f64::NAN, 0.0), 0);
        assert_eq!(append(-1, 0.0, 0.0, 0, 0.5), -1);
        assert!(coordinate(-1, 0, 0, 0).is_nan());
        let h = begin(0, 12.0, 0.25);
        assert!(h > 0);
        assert!(append(h, 10.0, 20.0, 0, 0.5) >= 0);
        let before = (0..4).map(|i| hash_word(h, i)).collect::<Vec<_>>();
        let p = preview(h, 40.0, 45.0, 10, 0.7);
        assert!(p > h);
        assert_eq!(before, (0..4).map(|i| hash_word(h, i)).collect::<Vec<_>>());
        assert_eq!(release(p), 0);
        assert_eq!(release(p), -1);
        assert_eq!(finish(h), 0);
        assert_eq!(before, (0..4).map(|i| hash_word(h, i)).collect::<Vec<_>>());
        assert_eq!(append(h, 1.0, 1.0, 20, 0.5), -1);
        assert_eq!(release(h), 0);
        let handles: Vec<_> = (0..MAX_HANDLES).map(|_| begin(0, 12.0, 0.0)).collect();
        assert!(handles.iter().all(|id| *id > p));
        assert_eq!(begin(0, 12.0, 0.0), 0);
        for id in handles {
            assert_eq!(release(id), 0);
        }
        assert_eq!(live_handles(), 0);
    }
}

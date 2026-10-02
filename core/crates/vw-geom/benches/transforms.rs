use criterion::{Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use vw_geom::{Affine, Point};

fn transforms(c: &mut Criterion) {
    let Ok(matrix) = Affine::new(1.5, 0.5, -0.5, 1.5, -2000.0, 1500.0) else {
        return;
    };
    let Ok(point) = Point::new(2431.125, -105.25) else {
        return;
    };
    c.bench_function("affine_map_checked_f64", |b| {
        b.iter(|| black_box(black_box(matrix).map(black_box(point))))
    });
}

criterion_group!(benches, transforms);
criterion_main!(benches);

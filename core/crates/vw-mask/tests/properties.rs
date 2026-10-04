#![allow(clippy::unwrap_used, clippy::expect_used)]
use proptest::prelude::*;
use vw_mask::*;

// Deliberately simple O(pixels * radius^2) references, independent of the core's
// sparse tiles, horizontal deques, scanlines and row-prefix implementation.
fn reference_disk(
    input: &[u8],
    width: usize,
    height: usize,
    radius: i32,
    operation: u8,
) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = if operation == 1 { 255u64 } else { 0 };
            let mut count = 0;
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy > radius * radius {
                        continue;
                    }
                    let px = x as i32 + dx;
                    let py = y as i32 + dy;
                    let sample = if px < 0 || py < 0 || px >= width as i32 || py >= height as i32 {
                        0
                    } else {
                        u64::from(input[py as usize * width + px as usize])
                    };
                    match operation {
                        0 => value = value.max(sample),
                        1 => value = value.min(sample),
                        _ => value += sample,
                    }
                    count += 1;
                }
            }
            output[y * width + x] = if operation == 2 {
                ((value + count / 2) / count) as u8
            } else {
                value as u8
            };
        }
    }
    output
}

fn clipped_area(points: &[Point], x: f64, y: f64) -> f64 {
    let mut polygon = points.to_vec();
    for (axis, edge, keep_greater) in [
        (0, x, true),
        (0, x + 1.0, false),
        (1, y, true),
        (1, y + 1.0, false),
    ] {
        let old = std::mem::take(&mut polygon);
        for (&a, &b) in old.iter().zip(old.iter().cycle().skip(1)).take(old.len()) {
            let av = if axis == 0 { a.x } else { a.y };
            let bv = if axis == 0 { b.x } else { b.y };
            let inside_a = if keep_greater { av >= edge } else { av <= edge };
            let inside_b = if keep_greater { bv >= edge } else { bv <= edge };
            if inside_a != inside_b {
                let t = (edge - av) / (bv - av);
                polygon.push(Point {
                    x: a.x + (b.x - a.x) * t,
                    y: a.y + (b.y - a.y) * t,
                });
            }
            if inside_b {
                polygon.push(b);
            }
        }
    }
    polygon
        .iter()
        .zip(polygon.iter().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a.x * b.y - a.y * b.x)
        .sum::<f64>()
        .abs()
        / 2.0
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, failure_persistence: None, max_shrink_iters: 1024, ..ProptestConfig::default() })]
    #[test]
    fn disk_filters_equal_independent_per_pixel_references(w in 1usize..11, h in 1usize..11, radius in 0u32..6, source in proptest::collection::vec(any::<u8>(),100)) {
        let input = &source[..w*h]; let mask = Mask::from_dense(Size::new(w as u32,h as u32).unwrap(), input).unwrap();
        prop_assert_eq!(mask.expand(radius).unwrap().to_dense().unwrap(), reference_disk(input,w,h,radius as i32,0));
        prop_assert_eq!(mask.shrink(radius).unwrap().to_dense().unwrap(), reference_disk(input,w,h,radius as i32,1));
        prop_assert_eq!(mask.feather(radius).unwrap().to_dense().unwrap(), reference_disk(input,w,h,radius as i32,2));
        prop_assert_eq!(mask.to_dense().unwrap(), input);
    }
    #[test]
    fn mask_algebra_matches_scalar_values_and_retains_history(a in proptest::collection::vec(any::<u8>(),16), b in proptest::collection::vec(any::<u8>(),16)) {
        let s = Size::new(4,4).unwrap(); let ma = Mask::from_dense(s,&a).unwrap(); let mb = Mask::from_dense(s,&b).unwrap();
        prop_assert_eq!(ma.add(&mb).unwrap().to_dense().unwrap(), a.iter().zip(&b).map(|(&x,&y)| x.max(y)).collect::<Vec<_>>());
        prop_assert_eq!(ma.intersect(&mb).unwrap().to_dense().unwrap(), a.iter().zip(&b).map(|(&x,&y)| x.min(y)).collect::<Vec<_>>());
        prop_assert_eq!(ma.subtract(&mb).unwrap().to_dense().unwrap(), a.iter().zip(&b).map(|(&x,&y)| x.saturating_sub(y)).collect::<Vec<_>>());
        prop_assert_eq!(ma.add(&mb).unwrap().content_hash(),mb.add(&ma).unwrap().content_hash());
        prop_assert_eq!(ma.intersect(&mb).unwrap().content_hash(),mb.intersect(&ma).unwrap().content_hash());
        prop_assert_eq!(ma.subtract(&ma).unwrap().tile_count(),0);
        prop_assert_eq!(ma.add(&ma).unwrap().content_hash(),ma.content_hash());
        prop_assert_eq!(ma.invert().unwrap().invert().unwrap().to_dense().unwrap(),a.clone());
        prop_assert_eq!(ma.version(),0);
        let encoded=ma.encode_lossless().unwrap(); prop_assert_eq!(Mask::decode_lossless(&encoded).unwrap().encode_lossless().unwrap(),encoded);
    }
    #[test]
    fn rectangle_and_lasso_match_independent_fractional_area(x in -8i32..16, y in -8i32..16, width in 0i32..24, height in 0i32..24) {
        let (x,y,width,height)=(f64::from(x)/4.0,f64::from(y)/4.0,f64::from(width)/4.0,f64::from(height)/4.0);
        let s=Size::new(5,5).unwrap();
        let rect=Mask::rectangle(s,Rect{x,y,width,height}).unwrap().to_dense().unwrap();
        let points=[Point{x,y},Point{x:x+width,y},Point{x:x+width,y:y+height},Point{x,y:y+height}];
        let lasso=Mask::lasso(s,&points).unwrap().to_dense().unwrap();
        for py in 0..5 { for px in 0..5 {
            let area=(f64::from(px+1).min(x+width)-f64::from(px).max(x)).max(0.0)*(f64::from(py+1).min(y+height)-f64::from(py).max(y)).max(0.0);
            let expected=(area*255.0+0.5) as u8; let index=(py*5+px) as usize;
            prop_assert_eq!(rect[index],expected); prop_assert!(lasso[index].abs_diff(expected)<=1);
        } }
    }
    #[test]
    fn triangle_scan_conversion_matches_analytic_clipped_polygon_area(coords in proptest::collection::vec(-16i32..48,6)) {
        let points=coords.as_chunks::<2>().0.iter().map(|pair|Point{x:f64::from(pair[0])/8.0,y:f64::from(pair[1])/8.0}).collect::<Vec<_>>();
        let pixels=Mask::lasso(Size::new(5,5).unwrap(),&points).unwrap().to_dense().unwrap();
        for y in 0..5 {for x in 0..5 {
            let expected=(clipped_area(&points,f64::from(x),f64::from(y))*255.0+0.5) as u8;
            prop_assert!(pixels[(y*5+x)as usize].abs_diff(expected)<=1);
        }}
    }
    #[test]
    fn feather_never_changes_pixels_outside_dilated_support(source in proptest::collection::vec(0u8..4,49), radius in 0u32..5) {
        let source=source.into_iter().map(|v|if v==3{255}else{0}).collect::<Vec<_>>();
        let mask=Mask::from_dense(Size::new(7,7).unwrap(),&source).unwrap();
        let support=mask.expand(radius).unwrap().to_dense().unwrap(); let blur=mask.feather(radius).unwrap().to_dense().unwrap();
        for (&support,&blur) in support.iter().zip(&blur) { if support==0 {prop_assert_eq!(blur,0);} }
    }
    #[test]
    fn malformed_lossless_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(),0..1024)) {
        if let Ok(mask)=Mask::decode_lossless(&bytes) {prop_assert_eq!(mask.encode_lossless().unwrap(),bytes);}
    }
}

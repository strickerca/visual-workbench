#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use vw_assets::*;

fn source(width: u32, height: u32) -> Source {
    Source {
        original: *blake3::hash(b"immutable-original").as_bytes(),
        original_bytes: 18,
        width,
        height,
        orientation: 1,
        bit_depth: 8,
        icc: None,
        display_conversion: false,
    }
}
fn manifest(width: u32, height: u32) -> Manifest {
    let s = source(width, height);
    let records = s
        .validate()
        .unwrap()
        .keys()
        .map(|key| TileRecord {
            key,
            encoded_bytes: 3,
            hash: *blake3::hash(b"abc").as_bytes(),
        })
        .collect();
    Manifest::new(s, records).unwrap()
}
fn view(x: f64, y: f64, w: f64, h: f64, scale: f64) -> Viewport {
    Viewport {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
        physical_scale: scale,
    }
}

#[test]
fn actual_200mp_dimensions_have_bounded_complete_odd_edge_pyramid() {
    let p = Pyramid::new(16_320, 12_240).unwrap();
    assert_eq!(p.tile_count(), 4105);
    assert_eq!(p.levels().last().unwrap().width, 1);
    assert_eq!(
        p.region(TileKey {
            level: 0,
            x: 63,
            y: 47
        })
        .unwrap(),
        TileRegion {
            x: 16_128,
            y: 12_032,
            width: 192,
            height: 208
        }
    );
    let odd = Pyramid::new(513, 257).unwrap();
    assert_eq!(odd.level(1).unwrap().width, 257);
    assert_eq!(odd.level(1).unwrap().height, 129);
    assert_eq!(odd.keys().count(), odd.tile_count());
}
#[test]
fn dimension_limits_refuse_before_tile_allocation() {
    for (w, h) in [(0, 1), (1, 0), (1_048_577, 1), (100_000, 100_000)] {
        assert!(Pyramid::new(w, h).is_err());
    }
}
#[test]
fn viewport_selects_native_density_and_exact_single_ring() {
    let p = Pyramid::new(2048, 2048).unwrap();
    let t = p.viewport(view(512., 512., 512., 512., 1.)).unwrap();
    assert_eq!(t.level, 0);
    assert_eq!(t.visible.len(), 4);
    assert_eq!(t.prefetch.len(), 12);
    assert!(!t.prefetch.iter().any(|k| t.visible.contains(k)));
    let half = p.viewport(view(0., 0., 1024., 1024., 0.5)).unwrap();
    assert_eq!(half.level, 1);
    assert_eq!(half.visible.len(), 4);
    assert!(
        p.viewport(view(-10., -10., 5., 5., 1.))
            .unwrap()
            .visible
            .is_empty()
    );
    assert!(p.viewport(view(0., 0., 1., 1., f64::NAN)).is_err());
}
#[test]
fn fractional_edges_do_not_fetch_neighbor_as_visible() {
    let t = Pyramid::new(1024, 1024)
        .unwrap()
        .viewport(view(256.1, 256.1, 255.9, 255.9, 2.))
        .unwrap();
    assert_eq!(
        t.visible,
        vec![TileKey {
            level: 0,
            x: 1,
            y: 1
        }]
    );
    assert_eq!(t.prefetch.len(), 8);
}
#[test]
fn fixed_manifest_roundtrip_bound_to_original_and_exact_inventory() {
    let m = manifest(513, 257);
    let b = m.encode();
    assert_eq!(Manifest::decode(&b, m.source()).unwrap(), m);
    let mut foreign = m.source().clone();
    foreign.original[0] ^= 1;
    assert!(Manifest::decode(&b, &foreign).is_err());
    let mut corrupt = b.clone();
    corrupt[20] ^= 1;
    assert!(Manifest::decode(&corrupt, m.source()).is_err());
    let mut trailing = b;
    trailing.push(0);
    assert!(Manifest::decode(&trailing, m.source()).is_err());
    assert_eq!(m.tiles()[0].relative_path(), "L0/0_0.webp");
    assert!(m.tiles().last().unwrap().relative_path().ends_with(".jpg"));
}
#[test]
fn manifest_count_bomb_is_rejected_before_allocation_even_with_valid_checksum() {
    let m = manifest(1, 1);
    let mut b = m.encode();
    b[92..96].copy_from_slice(&u32::MAX.to_le_bytes());
    let n = b.len() - 32;
    let h = blake3::hash(&b[..n]);
    b[n..].copy_from_slice(h.as_bytes());
    assert!(Manifest::decode(&b, m.source()).is_err());
}
#[test]
fn manifest_cannot_drop_duplicate_reorder_or_overclaim_tiles() {
    let m = manifest(257, 257);
    let mut records = m.tiles().to_vec();
    records.swap(0, 1);
    assert!(Manifest::new(m.source().clone(), records).is_err());
    let mut records = m.tiles().to_vec();
    records[0].encoded_bytes = MAX_ENCODED_TILE_BYTES as u32 + 1;
    assert!(Manifest::new(m.source().clone(), records).is_err());
    assert!(m.tiles()[0].verify(b"abd").is_err());
    assert!(m.tiles()[0].verify(b"abc").is_ok());
}
#[test]
fn display_profile_or_depth_conversion_must_be_explicit() {
    let mut s = source(1, 1);
    s.bit_depth = 16;
    assert!(s.validate().is_err());
    s.display_conversion = true;
    assert!(s.validate().is_ok());
    s.bit_depth = 8;
    s.display_conversion = false;
    s.icc = Some([1; 32]);
    assert!(s.validate().is_err());
}
#[test]
fn superseded_view_does_not_release_inflight_slots_or_accept_forged_receipt() {
    let mut scheduler = TileScheduler::new(manifest(4096, 4096), 1).unwrap();
    scheduler.viewport(view(0., 0., 256., 256., 1.)).unwrap();
    let r = scheduler.next(|_| false).unwrap().unwrap();
    scheduler
        .viewport(view(2048., 2048., 256., 256., 1.))
        .unwrap();
    assert!(scheduler.next(|_| false).unwrap().is_none());
    let mut fake = r;
    fake.manifest[0] ^= 1;
    assert!(scheduler.finish(fake).is_err());
    assert_eq!(scheduler.active().count(), 1);
    assert!(!scheduler.finish(r).unwrap());
    assert!(scheduler.finish(r).is_err());
    assert_eq!(
        scheduler.next(|_| false).unwrap().unwrap().tile,
        TileKey {
            level: 0,
            x: 8,
            y: 8
        }
    );
}
#[test]
fn repeated_tile_in_new_view_still_rejects_cancelled_completion() {
    let mut scheduler = TileScheduler::new(manifest(512, 512), 1).unwrap();
    scheduler.viewport(view(0., 0., 256., 256., 1.)).unwrap();
    let old = scheduler.next(|_| false).unwrap().unwrap();
    scheduler.viewport(view(0., 0., 256., 256., 1.)).unwrap();
    assert!(scheduler.next(|_| false).unwrap().is_none());
    assert!(!scheduler.finish(old).unwrap());
    let current = scheduler.next(|_| false).unwrap().unwrap();
    assert_eq!(current.tile, old.tile);
    assert_ne!(current.viewport, old.viewport);
    assert!(scheduler.finish(current).unwrap());
}
#[test]
fn priority_is_visible_before_prefetch_and_cached_tiles_never_request() {
    let mut s = TileScheduler::new(manifest(2048, 2048), 8).unwrap();
    let t = s.viewport(view(512., 512., 512., 512., 1.)).unwrap();
    for key in t.visible {
        assert_eq!(s.next(|_| false).unwrap().unwrap().tile, key);
    }
    let r = s.next(|_| true).unwrap();
    assert!(r.is_none());
}
#[test]
fn pins_and_decode_reservations_charge_actual_lifetimes() {
    let cache = BudgetCache::<u32, Vec<u8>>::new(16).unwrap();
    let lease = cache
        .reserve(1, 12)
        .unwrap()
        .commit(vec![1; 12], 12)
        .unwrap();
    assert_eq!(cache.accounting().unwrap(), (12, 0));
    assert!(matches!(cache.reserve(2, 8), Err(AssetError::Busy)));
    cache.clear().unwrap();
    assert!(cache.get(&1).unwrap().is_none());
    assert_eq!(cache.accounting().unwrap(), (12, 0));
    drop(lease);
    assert_eq!(cache.accounting().unwrap(), (0, 0));
    let pending = cache.reserve(3, 16).unwrap();
    assert_eq!(cache.accounting().unwrap(), (0, 16));
    cache.clear().unwrap();
    assert!(matches!(
        pending.commit(vec![2; 16], 16),
        Err(AssetError::Stale)
    ));
    assert_eq!(cache.accounting().unwrap(), (0, 0));
}
#[test]
fn live_cache_evicts_only_unleased_lru_and_failed_allocation_releases_reserve() {
    let cache = BudgetCache::<u32, Vec<u8>>::new(16).unwrap();
    drop(cache.reserve(1, 8).unwrap().commit(vec![1; 8], 8).unwrap());
    let pinned = cache.reserve(2, 8).unwrap().commit(vec![2; 8], 8).unwrap();
    let pending = cache.reserve(3, 8).unwrap();
    assert!(cache.get(&1).unwrap().is_none());
    assert_eq!(cache.accounting().unwrap(), (8, 8));
    drop(pending);
    assert_eq!(pinned.payload().unwrap(), &vec![2; 8]);
    assert_eq!(cache.accounting().unwrap(), (8, 0));
}
#[test]
fn concurrent_last_leases_release_retired_payload_exactly_once() {
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicUsize, Ordering},
    };
    struct Payload(Arc<AtomicUsize>);
    impl Drop for Payload {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let cache = BudgetCache::<u32, Payload>::new(16).unwrap();
    let first = cache
        .reserve(1, 16)
        .unwrap()
        .commit(Payload(drops.clone()), 16)
        .unwrap();
    let second = cache.get(&1).unwrap().unwrap();
    cache.clear().unwrap();
    let gate = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let a = gate.clone();
        scope.spawn(move || {
            a.wait();
            drop(first);
        });
        let b = gate.clone();
        scope.spawn(move || {
            b.wait();
            drop(second);
        });
        gate.wait();
    });
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(cache.accounting().unwrap(), (0, 0));
}

#[test]
fn anisotropic_and_sheared_views_use_largest_physical_scale() {
    let pyramid = Pyramid::new(4096, 4096).unwrap();
    let stretched = maximum_physical_scale(2.0, 0.0, 0.0, 0.25).unwrap();
    assert_eq!(stretched, 2.0);
    assert_eq!(
        pyramid
            .viewport(view(0., 0., 512., 512., stretched))
            .unwrap()
            .level,
        0
    );
    let reduced = maximum_physical_scale(0.125, 0.0, 0.0, 0.5).unwrap();
    assert_eq!(reduced, 0.5);
    assert_eq!(
        pyramid
            .viewport(view(0., 0., 512., 512., reduced))
            .unwrap()
            .level,
        1
    );
    // [[.25,.5],[0,.25]] has singular values (sqrt(2)+/-1)/4.
    let shear = maximum_physical_scale(0.25, 0.0, 0.5, 0.25).unwrap();
    assert!((shear - (2.0_f64.sqrt() + 1.0) / 4.0).abs() < 1e-12);
    assert_eq!(
        pyramid
            .viewport(view(0., 0., 512., 512., shear))
            .unwrap()
            .level,
        0
    );
    // A physical quarter-turn changes axes, never the density requirement.
    assert_eq!(
        maximum_physical_scale(0.0, 2.0, -0.25, 0.0).unwrap(),
        stretched
    );
}
#[test]
fn maximum_physical_scale_refuses_nonfinite_and_handles_extreme_finite_values() {
    assert_eq!(
        maximum_physical_scale(1e300, 0., 0., 1e-300).unwrap(),
        1e300
    );
    assert_eq!(
        maximum_physical_scale(1e-300, 0., 0., 1e-300).unwrap(),
        1e-300
    );
    assert!(maximum_physical_scale(0., 0., 0., 0.).is_err());
    assert!(maximum_physical_scale(f64::NAN, 0., 0., 1.).is_err());
    assert!(maximum_physical_scale(1., 0., 0., f64::INFINITY).is_err());
    assert!(maximum_physical_scale(f64::MAX, f64::MAX, f64::MAX, f64::MAX).is_err());
}

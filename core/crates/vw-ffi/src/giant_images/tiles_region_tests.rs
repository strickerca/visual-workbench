use super::*;

fn tile(x: u32, width: u32, color: u8) -> super::super::GiantTilePixels {
    super::super::GiantTilePixels {
        asset_id: "a".repeat(64),
        manifest_hash: "b".repeat(64),
        address: super::super::GiantTileAddress { level: 0, x, y: 0 },
        document_x: x * 256,
        document_y: 0,
        document_width: width,
        document_height: 1,
        width,
        height: 1,
        rgba: [color, 0, 0, 255].repeat(width as usize),
    }
}

#[test]
fn lossless_level_zero_seam_stitches_at_exact_d_coordinates() {
    let region = vw_raster::Region {
        x: 254,
        y: 0,
        width: 4,
        height: 1,
    };
    let cancel = Cancellation::new();
    let mut assembled = vec![0; 16];
    assert!(copy_original_tile(region, &tile(0, 256, 17), &mut assembled, &cancel).is_ok());
    assert!(copy_original_tile(region, &tile(1, 2, 41), &mut assembled, &cancel).is_ok());
    assert_eq!(
        assembled,
        vec![17, 0, 0, 255, 17, 0, 0, 255, 41, 0, 0, 255, 41, 0, 0, 255]
    );
}

#[test]
fn shifted_or_cancelled_level_zero_delivery_is_rejected() {
    let region = vw_raster::Region {
        x: 254,
        y: 0,
        width: 4,
        height: 1,
    };
    let cancel = Cancellation::new();
    let mut assembled = vec![0; 16];
    let mut shifted = tile(1, 2, 41);
    shifted.document_x = 255;
    assert!(matches!(
        copy_original_tile(region, &shifted, &mut assembled, &cancel),
        Err(Error::Invalid)
    ));
    cancel.cancel();
    assert!(matches!(
        copy_original_tile(region, &tile(0, 256, 17), &mut assembled, &cancel),
        Err(Error::Cancelled)
    ));
}

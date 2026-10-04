use vw_mask::{Mask, Point, Rect, Size};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let size = Size::new(19, 13)?;
    let rect = Mask::rectangle(
        size,
        Rect {
            x: 1.25,
            y: 0.5,
            width: 8.75,
            height: 9.25,
        },
    )?;
    let lasso = Mask::lasso(
        size,
        &[
            Point { x: -1.0, y: 3.25 },
            Point { x: 14.5, y: 1.0 },
            Point { x: 8.25, y: 12.75 },
            Point { x: 2.0, y: 7.0 },
        ],
    )?;
    let paint = Mask::paint(
        size,
        &[
            Point { x: 0.5, y: 11.5 },
            Point { x: 7.25, y: 2.25 },
            Point { x: 17.0, y: 9.0 },
        ],
        1.75,
        193,
    )?;
    let math = rect
        .add(&lasso)?
        .subtract(&paint)?
        .expand(2)?
        .feather(3)?
        .intersect(&lasso.invert()?)?;
    for (name, mask) in [
        ("rectangle", rect),
        ("lasso", lasso),
        ("paint", paint),
        ("combined", math),
    ] {
        println!(
            "{} v{} {}x{} {} {}",
            name,
            mask.version(),
            mask.size().width(),
            mask.size().height(),
            blake3::Hash::from(mask.content_hash()).to_hex(),
            blake3::hash(&mask.encode_lossless()?).to_hex()
        );
    }
    Ok(())
}

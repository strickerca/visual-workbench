//! Optional human inspection of synthetic fixtures. Supply a NEW directory
//! outside the repository, inspect the three PNGs, then dispose them. These are
//! verification images, never product assets or a substitute for golden tests.
#[path = "../tests/support/mod.rs"]
mod support;
use std::{fs::OpenOptions, io::Write, path::PathBuf};
use support::*;
use vw_raster::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or("supply a new inspection directory")?);
    if args.next().is_some() {
        return Err("one inspection directory is required".into());
    }
    std::fs::create_dir(&directory)?;
    let (original, project, doc) = all_annotations()?;
    let marked = render_document(&original, &project, &doc, &NoAssets, options())?;
    let mut white = original.clone();
    white.pixels = Pixels::Rgba16(vec![65535; original.pixels.len()]);
    let white_marked = render_document(&white, &project, &doc, &NoAssets, options())?;
    let mut req = request(ExportFormat::Png8);
    req.allow_depth_reduction = true;
    for (name, image) in [
        ("source", original),
        ("marked", marked),
        ("coverage", white_marked),
    ] {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(directory.join(format!("{name}.png")))?;
        file.write_all(&export(&image, &req)?.bytes)?;
        file.sync_all()?;
    }
    println!("Three synthetic verification PNGs written; inspect and dispose them.");
    Ok(())
}

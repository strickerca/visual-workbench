use super::*;
use std::collections::BTreeMap;
use vw_raster::*;
pub fn hashes() -> Result<BTreeMap<String, String>, Box<dyn std::error::Error>> {
    let mut hashes = BTreeMap::new();
    for depth in [8, 16] {
        let source = source(23, 17, depth);
        let format = if depth == 8 {
            ExportFormat::Png8
        } else {
            ExportFormat::Png16
        };
        let bytes = export(&source, &request(format))?.bytes;
        hashes.insert(
            format!("clean-png-{depth}"),
            vw_model::AssetId::hash(&bytes).to_string(),
        );
        let mut r = request(format);
        r.region = Some(Region {
            x: 3,
            y: 4,
            width: 7,
            height: 9,
        });
        let bytes = export(&source, &r)?.bytes;
        hashes.insert(
            format!("region-png-{depth}"),
            vw_model::AssetId::hash(&bytes).to_string(),
        );
    }
    let (source, project, doc) = all_annotations()?;
    let rendered = render_document(&source, &project, &doc, &NoAssets, options())?;
    for format in [ExportFormat::Png8, ExportFormat::Png16] {
        let mut r = request(format);
        r.allow_depth_reduction = true;
        let bytes = export(&rendered, &r)?.bytes;
        hashes.insert(
            format!("annotations-{format:?}"),
            vw_model::AssetId::hash(&bytes).to_string(),
        );
    }
    for font in [
        FontFamily::Inter,
        FontFamily::NotoSans,
        FontFamily::NotoSansMono,
    ] {
        let layout = layout_text("office AV café Ω Ж\n\u{202e}123\u{202c} xyz", font, 18.25)?;
        hashes.insert(
            format!("layout-{font:?}"),
            vw_model::AssetId::hash(&serde_json::to_vec(&layout)?).to_string(),
        );
    }
    Ok(hashes)
}

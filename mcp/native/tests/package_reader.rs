use vw_mcp_native::package_reader::{no_redirect, verify};
#[test]
fn incomplete_and_foreign_inventory_is_never_published() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    assert!(verify(root.path()).is_err());
    std::fs::write(root.path().join("manifest.json"), b"{}")?;
    assert!(verify(root.path()).is_err());
    std::fs::write(root.path().join("credentials.json"), b"synthetic-only")?;
    assert!(verify(root.path()).is_err());
    Ok(())
}
#[test]
fn declared_frame_budget_precedes_read_allocation() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let file = std::fs::File::create(root.path().join("manifest.json"))?;
    file.set_len(32 * 1024 * 1024 + 1)?;
    drop(file); // Close the fixture writer before the deny-write verifier opens it.
    assert_eq!(verify(root.path()), Err("package_limit"));
    Ok(())
}
#[test]
fn relative_and_parent_paths_are_refused() {
    assert!(no_redirect(std::path::Path::new("../private")).is_err());
}

#[test]
fn all_sixty_four_marker_crops_and_five_support_files_are_admitted()
-> Result<(), Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;
    use vw_instructions::{DocumentView, EditMetadata, EntryMethod, MarkerPlacement, Role};
    use vw_model::{AssetId, DeviceId, Document, Id, Layer, Project};
    use vw_package::{CompileInput, CompileOptions, Limits, NeverCancel, Target};
    use vw_proto::v1;
    fn id(n: u64) -> Result<Id, vw_model::ModelError> {
        Id::from_parts(9000 + n, [11; 10])
    }
    let mut original = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut original, 16, 16);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&[90u8, 120, 150, 255].repeat(256))?;
        writer.finish()?;
    }
    let device = DeviceId::from_bytes([0xa7; 16]);
    let document = id(2)?;
    let layer = id(3)?;
    let asset = AssetId::hash(&original);
    let mut project = Project::new(id(1)?, "synthetic".into(), device.clone());
    project.assets.insert(
        asset.clone(),
        v1::AddAsset {
            asset_id: asset.to_string(),
            format: "png".into(),
            width: 16,
            height: 16,
            orientation: 1,
            bit_depth: 8,
            has_alpha: false,
            color_space: "sRGB".into(),
            icc_profile: vec![],
            byte_size: original.len() as u64,
            source: "import".into(),
            captured_at_ms: 1,
            metadata_json: "{}".into(),
        },
    );
    project.documents.insert(
        document.clone(),
        Document {
            definition: v1::CreateDocument {
                document_id: Some(document.to_proto()),
                kind: v1::DocumentKind::Image as i32,
                schema_version: 1,
                title: "synthetic".into(),
                primary_asset_id: asset.to_string(),
                capture: None,
            },
            pages: vec![],
            created_at_ms: 1,
        },
    );
    project.layers.insert(
        layer.clone(),
        Layer {
            definition: v1::CreateLayer {
                layer_id: Some(layer.to_proto()),
                document_id: Some(document.to_proto()),
                page_index: -1,
                name: "Marks".into(),
                kind: "annotation".into(),
                order_key: "V".into(),
            },
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: "normal".into(),
        },
    );
    let mut host = vw_ops::HostSequencer::new(project, device.clone())?;
    for index in 0..64u64 {
        let view = DocumentView::new(host.project(), &host.revision()?, &document)?;
        let plan = view.place_marker(
            EditMetadata {
                transaction_id: id(1000 + index)?,
                device: device.clone(),
                first_lamport: 2000 + 512 * index,
                created_at_ms: 10,
            },
            MarkerPlacement {
                object_id: id(2000 + index)?,
                instruction_id: id(3000 + index)?,
                layer_id: layer.clone(),
                point: v1::PointD { x: 8.0, y: 8.0 },
                bounds: None,
                element_eids: vec![],
                style: v1::Style {
                    stroke: Some(v1::Color { rgba: 0xf03c32ff }),
                    width: 2.0,
                    ..Default::default()
                },
                role: Role::Change,
                text: format!("literal marker {}", index + 1),
                entry_method: EntryMethod::PhoneKeyboard,
                language: "en-US".into(),
            },
        )?;
        plan.submit(&mut host, &device, 11)?;
    }
    let originals = BTreeMap::from([(asset, original)]);
    let package = vw_package::compile(
        CompileInput {
            project: host.project(),
            revision: &host.revision()?,
            document: &document,
            originals: &originals,
        },
        CompileOptions {
            package_id: id(99)?,
            created_at_ms: 1_790_985_600_123,
            target: Target::Generic { max_long_edge: 256 },
            semantic_snapshot: None,
            include_window_title: false,
            assume_untagged_srgb: true,
            allow_depth_reduction: false,
            limits: Limits::default(),
        },
        &NeverCancel,
    )?;
    assert_eq!(package.files().count(), 69);
    let root = tempfile::tempdir()?;
    std::fs::create_dir(root.path().join("images"))?;
    for (name, bytes) in package.files() {
        std::fs::write(root.path().join(name), bytes)?;
    }
    let verified = verify(root.path())?;
    assert_eq!(verified["manifest_sha256"], package.manifest_sha256());
    assert_eq!(
        verified["manifest"]["markers"]
            .as_array()
            .ok_or("markers")?
            .len(),
        64
    );
    Ok(())
}

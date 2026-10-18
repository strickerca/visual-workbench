use std::{env, error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let contracts = manifest.join("../../../contracts");
    let schema = contracts.join("vw_protocol.proto");
    println!("cargo:rerun-if-changed={}", schema.display());
    let mut config = prost_build::Config::new();
    config.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
    config.btree_map(["."]);
    // Keep the largest draft envelope payload indirect without changing its
    // protobuf field number, serialized bytes, or the model's ObjectState type.
    config.boxed(".vw.v1.Envelope.body.gesture_update");
    config.boxed(".vw.v1.Envelope.body.gesture_replay");
    config.type_attribute(".", "#[derive(::serde::Serialize, ::serde::Deserialize)]");
    config.message_attribute(".", "#[serde(deny_unknown_fields)]");
    config.compile_protos(&[schema], &[contracts])?;
    Ok(())
}

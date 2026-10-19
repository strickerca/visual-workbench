use crate::{CoreError, ProjectInfo};
pub(crate) fn export_receipt(
    metadata: &vw_raster::ExportMetadata,
    document_id: &str,
    revision: &ProjectInfo,
) -> crate::Result<Option<String>> {
    let Some(capture) = &metadata.settings.capture_provenance else {
        return Ok(None);
    };
    vw_model::validate_capture(capture)?;
    if metadata.settings.revision.host_seq != revision.host_seq
        || metadata
            .settings
            .revision
            .state_hash
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            != revision.state_hash
        || capture
            .stream_save
            .as_ref()
            .is_some_and(|s| s.saved_asset_id != metadata.source_asset)
    {
        return Err(CoreError::Invalid);
    }
    Ok(Some(
        serde_json::to_string(&serde_json::json!({
            "schema":1,"project_id":revision.project_id,"document_id":document_id,
            "host_seq":revision.host_seq,"state_hash":revision.state_hash,
            "source_asset":metadata.source_asset,"capture":capture,
        }))
        .map_err(|_| CoreError::Invalid)?,
    ))
}
